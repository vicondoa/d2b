//! The legacy driver guest-control surface.
//!
//! A check that has not been ported yet is a Python script and has to keep
//! gating the lane, so the lane owes that script the same guest it had: the
//! `machine` object the nix test driver gave it, and the diagnostics prelude
//! its `testScript` opens with. This module is the first of those and the
//! crate owns the second, in one file ([`DIAGNOSTICS`]) that the nix lane's
//! fixtures interpolate as well, so a check's failure reads the same before
//! and after its port.
//!
//! A check that *has* been ported asserts in Rust (`crate::checks`), against
//! this same surface rather than against a second one: the operations, their
//! bounds, their wording, and the diagnostics the prelude prints are the
//! ones here, so one check's port moves its assertions and nothing else. An
//! unported check reaches them through the bridge; a ported one calls them
//! directly, and [`LegacyGuest::run_ported`] reports it the way
//! [`LegacyGuest::run`] reports a script.
//!
//! The shape of the surface is the driver's, deliberately. Every operation a
//! fixture calls is here with the driver's semantics: `execute` runs a
//! command under `set -euo pipefail` with the bound the check declared,
//! `succeed` and `fail` are the two assertions on its exit status,
//! `wait_until_succeeds` retries it on the driver's interval, `wait_for_unit`
//! and `wait_for_file` are the two service and file waits, and each one
//! reports in the driver's own words - a refused assertion reads
//! `command \`…\` failed (exit code N)` and a timed-out wait reads
//! `action timed out after X seconds (timeout=N)`, because the lane's
//! diagnostics prelude prints that message and a differently worded refusal
//! is a differently reported failure for the same failure.
//!
//! Two things are worth naming about how it is built.
//!
//! * The guest is reached over the console the legacy driver itself used: a
//!   virtio serial console carrying a root shell, spoken to with the driver's
//!   own base64-and-`PIPESTATUS` framing. Nothing else is required of the
//!   guest: a guest that runs the nix test framework's `backdoor.service` (or
//!   any root shell that announces itself with the line below and reads
//!   commands from that console) is a guest this surface can drive. It is not
//!   a network port, so it survives a snapshot and a restore, and it does not
//!   put an ssh client between a check's assertion and the command it ran.
//! * The operations live here rather than in the Python that calls them. The
//!   check's script is executed by a Python interpreter, but every operation
//!   it calls is a request to this module, so the semantics of an operation -
//!   the shell a command runs under, the retry bounds, the wording of a
//!   refusal - are implemented once and are the same whether the caller is an
//!   unported Python check or a ported Rust one.

use std::{
    collections::BTreeMap,
    env,
    fmt, fs,
    io::{self, BufRead, Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    checks::Assertions,
    error::{HarnessError, Result},
    guest::{ActiveGuest, CONSOLE_ID, report},
};

/// The lane's fixture diagnostics prelude, in the one place it lives.
///
/// The nix lane interpolates this same file into every fixture's evaluated
/// `testScript` (`tests/host-integration/lib.nix` reads it), and the lane runs
/// that evaluated script, so the diagnostics an unported check prints are
/// printed from this text rather than from a second copy of it. A check that
/// ports keeps reporting through it.
pub const DIAGNOSTICS: &str = include_str!("diagnostics.py");

/// The Python side of the surface: the `machine` object an unported check's
/// assertions call, and nothing else.
const BRIDGE: &str = include_str!("legacy_bridge.py");

/// The line the guest's root shell announces itself with, exactly as the nix
/// test framework's backdoor service announces it. The greeting is part of
/// the protocol rather than a courtesy: the driver waits for it before it
/// sends anything, because the console is a shell that may still be sourcing
/// a profile when the connection lands.
const SHELL_GREETING: &str = "Spawning backdoor root shell...";

/// How long the guest's shell has to announce itself after the console is
/// attached. The guest has already reported activation before the surface
/// attaches, so this bound is about the console rather than about the boot.
const SHELL_GREETING_TIMEOUT: Duration = Duration::from_secs(300);

/// The interval between attempts of a retrying operation, the driver's own.
const RETRY_INTERVAL: Duration = Duration::from_secs(1);

/// The bound `execute` applies when a caller names none - which is every
/// `wait_for_file` attempt, because the driver reached `execute` through the
/// default rather than through a declared one. Restated here because the
/// Python side resolves the same default for the calls it forwards, and two
/// declarations of the driver's default would be two chances to disagree.
const EXECUTE_DEFAULT_TIMEOUT: u64 = 900;

/// The bound one diagnostic command gets, the prelude's own. Diagnostics are
/// read on the failure path, so they are bounded twice over: by this, and by
/// the `timeout` the prelude wraps the zone explanation in.
const DIAGNOSTIC_TIMEOUT: u64 = 120;

/// The zone and the linux user a failure's composed explanation is read
/// through. Every fixture drives one zone as one user through the same public
/// socket, so `d2b debug` explains the whole zone without the failing stage
/// listing the rows it asserted on; these are the prelude's `_diag_zone` and
/// `_diag_user`, restated in the one place a ported check's diagnostics read
/// them from.
const DIAG_ZONE: &str = "work";
const DIAG_USER: &str = "alice";

/// One row a stage was asserting on: the label it is reported under, and the
/// command that dumps it.
pub type DiagRow<'a> = (&'a str, &'a str);

/// How often a check's connection to the harness is looked for, while
/// refusing to block on a check that has already exited.
const ACCEPT_POLL: Duration = Duration::from_millis(20);

/// The environment variable that names the interpreter a check's script runs
/// under, for a lane that wants a specific one. The lane's test target sets
/// it to the interpreter declared as a runfile; without it the interpreter is
/// resolved from the runfiles tree, and failing that from `PATH`.
const PYTHON: &str = "D2B_TEST_VM_HARNESS_PYTHON";

/// The smallest read bound the console is given while it waits for its
/// shell. A bound of zero is not a bound at all on a socket: it means wait
/// forever.
const MINIMUM_READ_BOUND: Duration = Duration::from_millis(1);

/// One check that has not been ported: its name, and its evaluated
/// `testScript` - the fixture's own assertions with the shared diagnostics
/// prelude already interpolated at the top.
///
/// The script is whatever the fixture evaluated to, not the fixture file: the
/// lane reads the check out of the same evaluation that produced its guest
/// image, so the assertions a check runs here are the assertions it runs
/// under the driver being retired.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyCheck {
    /// The check's name, as it appears in the lane's own results.
    pub name: String,
    /// The check's script.
    pub script: String,
}

impl LegacyCheck {
    /// Take a check as its name and its evaluated script.
    pub fn new(name: impl Into<String>, script: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            script: script.into(),
        }
    }
}

/// What one unported check produced.
///
/// The diagnostics themselves are already in the lane's report by the time
/// this is returned - the check's own output and the surface's log lines are
/// streamed as they happen, in the order they happened - and `detail` carries
/// them again for the caller that files them under this check's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyOutcome {
    /// The check's name.
    pub name: String,
    /// Whether the check's script finished without a failure.
    pub passed: bool,
    /// Everything the check printed, and everything the surface reported
    /// around it.
    pub detail: String,
}

/// Why a guest-control operation did not produce a result.
///
/// The two cases are kept apart because they are the check's verdict and the
/// lane's, and an unported check is the only one that can fix the first: a
/// refused assertion is the check's own failure and travels to the check's
/// script, while a guest the surface could not reach is a lane failure that
/// no assertion in the check caused.
#[derive(Debug)]
pub enum LegacyError {
    /// The guest could not be reached, or answered something unreadable.
    Guest(HarnessError),
    /// An assertion the check made did not hold.
    Assertion(String),
}

impl fmt::Display for LegacyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Guest(error) => write!(formatter, "{error}"),
            Self::Assertion(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for LegacyError {}

impl From<HarnessError> for LegacyError {
    fn from(error: HarnessError) -> Self {
        Self::Guest(error)
    }
}

/// The guest-control surface's own result type.
pub type LegacyResult<T> = std::result::Result<T, LegacyError>;

/// What one command did in the guest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResult {
    /// The command's exit status.
    pub status: i32,
    /// Everything it wrote, on either stream.
    pub output: String,
}

/// Quote a string the way the guest's shell needs it quoted.
///
/// The rules are the interpreter's own rather than a shell's, because the
/// string is handed to `bash -c` as one argument and a different rule would
/// change what the guest runs for a command containing a quote: an ASCII
/// string of unreserved characters passes through, and anything else is
/// single-quoted with an embedded quote closed, double-quoted, and reopened.
fn shlex_quote(text: &str) -> String {
    if text.is_empty() {
        return "''".to_owned();
    }
    let unreserved = |byte: u8| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'%' | b'+' | b',' | b'-' | b'.' | b'/' | b':' | b'=' | b'@')
    };
    if text.is_ascii() && text.bytes().all(unreserved) {
        return text.to_owned();
    }
    format!("'{}'", text.replace('\'', "'\"'\"'"))
}

/// Decode one base64 block, the framing the guest's shell answers with.
///
/// The block is ASCII by construction, so the decode is a table lookup per
/// character and a shift-and-or per four. Whitespace is skipped rather than
/// rejected: the framing wraps at the shell's own line width on some
/// guests, and padding is what ends a well-formed block.
fn base64_decode(block: &str) -> std::result::Result<Vec<u8>, HarnessError> {
    const INVALID: u8 = u8::MAX;
    let value = |byte: u8| -> u8 {
        match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => INVALID,
        }
    };
    let mut out = Vec::with_capacity(block.len() / 4 * 3);
    let mut accumulator: u32 = 0;
    let mut bits: u32 = 0;
    for byte in block.bytes() {
        match byte {
            b'=' | b'\n' | b'\r' | b' ' | b'\t' => continue,
            byte => {
                let digit = value(byte);
                if digit == INVALID {
                    return Err(HarnessError::Configuration(
                        "the guest answered a command with output that is not base64".to_owned(),
                    ));
                }
                accumulator = (accumulator << 6) | u32::from(digit);
                bits += 6;
                if bits >= 8 {
                    bits -= 8;
                    out.push(((accumulator >> bits) & 0xff) as u8);
                }
            }
        }
    }
    Ok(out)
}

/// The guest-side shell, spoken to the way the legacy driver spoke to it.
///
/// One connection, one request at a time, which is the shape the driver had:
/// a check's assertions are sequential, and a shell shared by concurrent
/// readers would interleave two commands' output into one block.
struct Console {
    reader: io::BufReader<UnixStream>,
    writer: UnixStream,
    work_dir: PathBuf,
}

impl Console {
    /// Take a booted guest's command channel and wait for its shell.
    ///
    /// The channel is already declared on the guest's launch and the host end
    /// of it is already listening, because the guest's half is a unit that
    /// requires `dev-hvc0.device` to exist before the guest boots: a console
    /// added to a running guest would leave a guest whose root shell systemd
    /// never started. What is left here is accepting the connection the
    /// emulator made and waiting for the shell, which has to be running by
    /// the time the surface finishes attaching - hence a bounded wait that
    /// names what it was waiting for.
    fn attach(guest: &mut ActiveGuest) -> Result<Self> {
        let work_dir = guest.work_dir().to_path_buf();
        let listener = guest.take_command_channel()?;
        let stream = accept(&listener, None)?;
        let mut console = Self {
            reader: io::BufReader::new(
                stream
                    .try_clone()
                    .map_err(|error| HarnessError::io("cloning the console socket", error))?,
            ),
            writer: stream,
            work_dir,
        };
        console.await_shell(SHELL_GREETING_TIMEOUT)?;
        report(&format!(
            "the lane's guest-control console is attached to the guest's {CONSOLE_ID} chardev"
        ));
        Ok(console)
    }

    /// Take a surface over a console that is already being served.
    ///
    /// The lane attaches a console to a guest it booted; a test stands one up
    /// in place of a guest, over a socket pair, and drives the same
    /// operations against it.
    #[cfg(test)]
    fn serving(stream: UnixStream, work_dir: PathBuf) -> Self {
        Self {
            reader: io::BufReader::new(
                stream
                    .try_clone()
                    .expect("a console socket can be cloned for reading"),
            ),
            writer: stream,
            work_dir,
        }
    }

    /// Wait for the guest's shell to announce itself.
    ///
    /// The greeting is what tells the surface the shell is reading the
    /// console rather than still being set up on it, so a console that never
    /// greets is a guest with no shell service rather than a slow one. The
    /// bound is therefore applied to the reads as well as to the clock: a
    /// console with nothing to say is a read that would not return, and
    /// bounding the read is what turns the first into the message below
    /// rather than into a wait.
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    fn await_shell(&mut self, bound: Duration) -> Result<()> {
        self.writer
            .set_read_timeout(Some(bound.max(MINIMUM_READ_BOUND)))
            .map_err(|error| HarnessError::io("bounding the console's read", error))?;
        let mut seen = String::new();
        let start = Instant::now();
        let greeting = loop {
            if seen.contains(SHELL_GREETING) {
                break true;
            }
            if start.elapsed() >= bound {
                break false;
            }
            let mut chunk = [0_u8; 4096];
            match self.reader.read(&mut chunk) {
                Ok(0) => break false,
                Ok(read) => seen.push_str(&String::from_utf8_lossy(&chunk[..read])),
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::Interrupted
                            | io::ErrorKind::WouldBlock
                            | io::ErrorKind::TimedOut
                    ) => {}
                Err(error) => {
                    return Err(HarnessError::io("reading the guest's console", error));
                }
            }
        };
        // A command's own bound is the guest's, and a read that had a bound
        // of its own would cut a command off before the guest's bound did.
        self.writer
            .set_read_timeout(None)
            .map_err(|error| HarnessError::io("unbounding the console's read", error))?;
        if greeting {
            return Ok(());
        }
        Err(HarnessError::Configuration(format!(
            "the guest's console never announced its root shell with {SHELL_GREETING:?} within {}s, so the lane has no way to run a check's assertions: the guest must run a root shell on a virtio serial console ({SHELL_GREETING}), and it must be running by the time the surface attaches the console",
            bound.as_secs()
        )))
    }

    /// Wait for the guest's shell to announce itself again, discarding
    /// whatever the console is holding.
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    fn resync(&mut self, bound: Duration) -> Result<()> {
        self.await_shell(bound)
    }

    /// Run one command in the guest and read back its status and output.
    ///
    /// The wire form is the driver's, unchanged: the command is run under
    /// `set -euo pipefail` so a check's own shell assumptions - a pipeline
    /// that fails, an unset variable - fail the way they failed for it, its
    /// output is base64-framed so a block with a newline in it is still one
    /// block, and its status is read from the pipeline's own `PIPESTATUS`
    /// rather than from the status of the framing around it.
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    fn run(&mut self, command: &str, timeout: Option<u64>) -> Result<CommandResult> {
        let deadline = timeout.map(|seconds| format!("timeout {seconds} ")).unwrap_or_default();
        let inner = format!("set -euo pipefail; {command}");
        self.send(&format!(
            "{deadline}bash -c {} | (base64 -w 0; echo)\n",
            shlex_quote(&inner)
        ))?;
        let output = base64_decode(self.read_block()?.trim())?;
        self.send("echo ${PIPESTATUS[0]}\n")?;
        let status = self.read_block()?;
        let status = status.trim().parse::<i32>().map_err(|error| {
            HarnessError::Configuration(format!("the guest answered {status:?} as a status: {error}"))
        })?;
        Ok(CommandResult {
            status,
            output: String::from_utf8_lossy(&output).into_owned(),
        })
    }

    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    fn send(&mut self, wire: &str) -> Result<()> {
        self.writer
            .write_all(wire.as_bytes())
            .map_err(|error| HarnessError::io("writing to the guest's console", error))
    }

    /// Read one newline-terminated block from the console.
    ///
    /// The block ends at the newline the shell's framing adds, which is the
    /// only newline in it: the output itself is base64, so it carries none.
    /// A read that returns nothing at all is a guest that stopped answering,
    /// and is reported as such rather than as an empty output.
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    fn read_block(&mut self) -> Result<String> {
        let mut block = String::new();
        let mut chunk = [0_u8; 4096];
        loop {
            let read = self
                .reader
                .read(&mut chunk)
                .map_err(|error| HarnessError::io("reading the guest's console", error))?;
            if read == 0 {
                return Err(HarnessError::Configuration(
                    "the guest's console closed while a command was running".to_owned(),
                ));
            }
            let decoded = String::from_utf8_lossy(&chunk[..read]);
            block.push_str(&decoded);
            if decoded.ends_with('\n') {
                return Ok(block);
            }
        }
    }
}

/// Take a connection the lane is waiting for, without waiting forever for one
/// that is never coming.
///
/// A check's interpreter can die before it asks for the guest at all - the
/// interpreter is missing, the script does not parse - and a blocking accept
/// would then sit on a wait whose only other outcome is the lane's own
/// timeout. The process the connection is expected from is therefore watched
/// while the wait runs, and a process that has already exited ends it.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn accept(listener: &UnixListener, mut check: Option<&mut Child>) -> Result<UnixStream> {
    listener
        .set_nonblocking(true)
        .map_err(|error| HarnessError::io("making a lane socket non-blocking", error))?;
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Ok(stream),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if let Some(check) = check.as_deref_mut()
                    && let Some(status) = check.try_wait().map_err(|error| {
                        HarnessError::io("asking whether the check's script is still running", error)
                    })?
                {
                    return Err(HarnessError::Spawn {
                        detail: format!(
                            "the check's interpreter exited with {status} before it asked for the guest"
                        ),
                    });
                }
                thread::sleep(ACCEPT_POLL);
            }
            Err(error) => return Err(HarnessError::io("accepting a lane connection", error)),
        }
    }
}

/// The guest-control surface an unported check's assertions call.
///
/// Every operation here is one the fixtures call, with the driver's
/// semantics; the log lines around them are the driver's, so a lane log
/// reads the way the lane log read while the driver was the thing driving
/// the guest.
pub struct GuestControl {
    console: Console,
    notes: String,
    diagnostics: Diagnostics,
}

/// Where a check's diagnostics stand: the stage it is in, and the moment its
/// lines are timed against.
///
/// The prelude keeps the same two facts in its own module state, because an
/// unported check's script is the thing being timed. A ported check's
/// assertions are this surface's calls, so the two live here instead, and a
/// check starts them with [`GuestControl::begin_check`] - a check's lines are
/// timed from the check's own beginning, the way the prelude's are timed from
/// the script's.
struct Diagnostics {
    stage: String,
    started: Instant,
}

impl Diagnostics {
    fn new() -> Self {
        Self {
            stage: "startup".to_owned(),
            started: Instant::now(),
        }
    }

    /// The elapsed time, rendered the way the prelude renders it.
    fn elapsed(&self) -> String {
        format!("{:.1}s", self.started.elapsed().as_secs_f64())
    }
}

impl GuestControl {
    /// The surface over one console, with nothing reported yet.
    ///
    /// The console is the only thing that has to be supplied: the report a
    /// check accumulates is empty until it starts, and so are the diagnostics
    /// that time it.
    fn new(console: Console) -> Self {
        Self {
            console,
            notes: String::new(),
            diagnostics: Diagnostics::new(),
        }
    }

    /// Run a command in the guest.
    ///
    /// `timeout` is the bound the command's own execution gets, in seconds;
    /// `None` is the driver's unbounded form. The status and the output are
    /// returned as they are, and it is the caller's assertion that judges
    /// them: `execute` is the one operation here that refuses nothing.
    pub fn execute(&mut self, command: &str, timeout: Option<u64>) -> LegacyResult<CommandResult> {
        Ok(self.console.run(command, timeout)?)
    }

    /// Run each command in turn, refusing anything that exits non-zero.
    ///
    /// The outputs are concatenated, as the driver concatenated them, because
    /// a check that passes several commands and reads the result is reading
    /// one string.
    pub fn succeed(&mut self, commands: &[&str], timeout: Option<u64>) -> LegacyResult<String> {
        let mut output = String::new();
        for command in commands {
            let message = format!("must succeed: {command}");
            self.note(&message);
            let started = Instant::now();
            let result = self.execute(command, timeout)?;
            if result.status != 0 {
                self.note(&format!("output: {}", result.output.trim_end()));
                return Err(LegacyError::Assertion(format!(
                    "command `{command}` failed (exit code {})",
                    result.status
                )));
            }
            output.push_str(&result.output);
            self.finished(&message, started);
        }
        Ok(output)
    }

    /// Run each command in turn, refusing anything that exits zero.
    pub fn fail(&mut self, commands: &[&str], timeout: Option<u64>) -> LegacyResult<String> {
        let mut output = String::new();
        for command in commands {
            let message = format!("must fail: {command}");
            self.note(&message);
            let started = Instant::now();
            let result = self.execute(command, timeout)?;
            if result.status == 0 {
                return Err(LegacyError::Assertion(format!(
                    "command `{command}` unexpectedly succeeded"
                )));
            }
            output.push_str(&result.output);
            self.finished(&message, started);
        }
        Ok(output)
    }

    /// Retry a command until it succeeds, and return the output of the
    /// attempt that succeeded.
    ///
    /// A wait that runs out reports the bound and the driver's message, and
    /// then reports the output the last attempt saw - in the driver's own
    /// `output:` idiom, the one it logged beside a refused command. That last
    /// observation is the reason the lane's diagnostics prelude exists, and
    /// losing it again here would put it back where the driver left it.
    pub fn wait_until_succeeds(&mut self, command: &str, bound: Duration) -> LegacyResult<String> {
        let message = format!("waiting for success: {command}");
        self.note(&message);
        let started = Instant::now();
        let mut last = String::new();
        let attempt_bound = Some(bound.as_secs());
        let outcome = self.retry(bound, |control| {
            let result = control.execute(command, attempt_bound)?;
            last = result.output;
            Ok(result.status == 0)
        });
        match outcome {
            Ok(()) => {
                self.finished(&message, started);
                Ok(last)
            }
            Err(error) => {
                self.note(&format!("output: {}", last.trim_end()));
                Err(error)
            }
        }
    }

    /// Wait until a path exists in the guest.
    ///
    /// Each attempt is a `test -e` under the default execution bound rather
    /// than the wait's own, which is what the driver did: the wait bounds how
    /// long the attempts go on for, and each attempt bounds one command.
    pub fn wait_for_file(&mut self, path: &str, bound: Duration) -> LegacyResult<()> {
        let message = format!("waiting for file '{path}'");
        self.note(&message);
        let started = Instant::now();
        let command = format!("test -e {path}");
        let outcome = self.retry(bound, |control| {
            Ok(control.execute(&command, Some(EXECUTE_DEFAULT_TIMEOUT))?.status == 0)
        });
        if outcome.is_ok() {
            self.finished(&message, started);
        }
        outcome
    }

    /// Wait until a systemd unit is active.
    ///
    /// Two states end the wait early rather than being waited out, because a
    /// unit that failed or that is inactive with nothing left to do is not
    /// going to become active and the reader of the lane log needs to know
    /// that now. The second of the two is the driver's own: "no jobs" is how
    /// the guest says it has nothing left in flight for the unit.
    pub fn wait_for_unit(
        &mut self,
        unit: &str,
        user: Option<&str>,
        bound: Duration,
    ) -> LegacyResult<()> {
        let message = match user {
            Some(user) => format!("waiting for unit {unit} with user {user}"),
            None => format!("waiting for unit {unit}"),
        };
        self.note(&message);
        let started = Instant::now();
        let outcome = self.retry(bound, |control| control.unit_is_active(unit, user));
        if outcome.is_ok() {
            self.finished(&message, started);
        }
        outcome
    }

    /// Sleep in guest time, the way the driver's sleep was guest time: the
    /// command runs in the guest, so a guest whose clock runs at a different
    /// rate still sleeps for the seconds the check asked for.
    pub fn sleep(&mut self, seconds: u64) -> LegacyResult<()> {
        self.succeed(&[&format!("sleep {seconds}")], None)?;
        Ok(())
    }

    /// Announce the stage a check is in.
    ///
    /// A ported check calls this where its fixture called the prelude's
    /// `stage`, so a failure names the phase it happened in, in the line the
    /// fixture's failure named it in, timed from the check's own start.
    pub fn stage(&mut self, name: &str) {
        self.diagnostics.stage = name.to_owned();
        let line = format!("[d2b] stage={name} t={}", self.diagnostics.elapsed());
        self.announce(&line);
    }

    /// Run a diagnostic command and report what it wrote.
    ///
    /// Diagnostics only, exactly as the prelude's `diag` is: the status is
    /// returned rather than asserted on, and a diagnostic that could not run
    /// at all is reported as the prelude reported it rather than becoming an
    /// error of its own - a failure that happened before the guest could
    /// answer must still print its own stage.
    pub fn diag(&mut self, command: &str, label: &str) -> i32 {
        match self.execute(command, Some(DIAGNOSTIC_TIMEOUT)) {
            Err(error) => {
                let stage = self.diagnostics.stage.clone();
                let line = format!(
                    "[d2b] stage={stage} t={} {label}: diagnostic command failed: {error}",
                    self.diagnostics.elapsed(),
                );
                self.announce(&line);
                -1
            }
            Ok(result) => {
                let stage = self.diagnostics.stage.clone();
                let head = format!(
                    "[d2b] stage={stage} t={} {label} (exit {}):",
                    self.diagnostics.elapsed(),
                    result.status,
                );
                self.announce(&head);
                self.announce(command);
                for line in result.output.trim_end().lines() {
                    self.announce(&format!("    {line}"));
                }
                result.status
            }
        }
    }

    /// Wait for a unit, and report everything that explains a wait that did
    /// not finish.
    ///
    /// The wait itself is [`Self::wait_for_unit`] with no user and the
    /// check's own bound, which is what the prelude's `diag_unit` was. What
    /// this adds is the failure path: the stage it was in, the unit's status
    /// dump, the unit's own journal, and the zone's composed explanation, in
    /// the prelude's own order and wording.
    pub fn diag_unit(&mut self, stage: &str, unit: &str, bound: Duration) -> LegacyResult<()> {
        self.stage(stage);
        match self.wait_for_unit(unit, None, bound) {
            Ok(()) => Ok(()),
            Err(error) => {
                let label = format!("{unit} status");
                let dump = format!("systemctl status {unit} --no-pager 2>&1 | tail -n 40 || true");
                self.explain_failure(stage, None, &[(label.as_str(), dump.as_str())], &[(unit, "")], &error);
                Err(error)
            }
        }
    }

    /// Wait for a command to succeed, and report the same explanation on
    /// failure, with the wait itself named.
    ///
    /// `rows` are the resource rows the stage was asserting on and `explain`
    /// are the journal sources that explain them, as `(unit, token)` - an
    /// empty unit is the whole journal and an empty token is no filter - which
    /// is the prelude's `diag_wait` shape.
    pub fn diag_wait(
        &mut self,
        stage: &str,
        command: &str,
        bound: Duration,
        rows: &[DiagRow<'_>],
        explain: &[DiagRow<'_>],
    ) -> LegacyResult<String> {
        self.stage(stage);
        match self.wait_until_succeeds(command, bound) {
            Ok(output) => Ok(output),
            Err(error) => {
                self.explain_failure(stage, Some(command), rows, explain, &error);
                Err(error)
            }
        }
    }

    /// Whether a unit is active, and the two states that end a wait early.
    fn unit_is_active(&mut self, unit: &str, user: Option<&str>) -> LegacyResult<bool> {
        let state = self.unit_property(unit, "ActiveState", user)?;
        if state == "failed" {
            return Err(LegacyError::Assertion(format!(
                "unit \"{unit}\" reached state \"{state}\""
            )));
        }
        if state == "inactive" {
            let jobs = self.systemctl("list-jobs --full 2>&1", user)?;
            if jobs.output.contains("No jobs")
                && self.unit_info(unit, user)?.get("ActiveState") == Some(&state)
            {
                return Err(LegacyError::Assertion(format!(
                    "unit \"{unit}\" is inactive and there are no pending jobs"
                )));
            }
        }
        Ok(state == "active")
    }

    /// One systemd property of one unit.
    fn unit_property(
        &mut self,
        unit: &str,
        property: &str,
        user: Option<&str>,
    ) -> LegacyResult<String> {
        let under_user = match user {
            Some(user) => format!(" under user \"{user}\""),
            None => String::new(),
        };
        let result = self.systemctl(
            &format!("--no-pager show \"{unit}\" --property=\"{property}\""),
            user,
        )?;
        if result.status != 0 {
            return Err(LegacyError::Assertion(format!(
                "retrieving systemctl property \"{property}\" for unit \"{unit}\"{under_user} failed with exit code {}",
                result.status
            )));
        }
        let invalid = || {
            LegacyError::Assertion(format!(
                "systemctl show --property \"{property}\" \"{unit}\" produced invalid output: {}",
                result.output
            ))
        };
        let first = result.output.split('\n').next().unwrap_or_default();
        let (key, value) = first.split_once('=').ok_or_else(invalid)?;
        if key != property {
            return Err(invalid());
        }
        Ok(value.to_owned())
    }

    /// Every property the guest reports for one unit.
    fn unit_info(&mut self, unit: &str, user: Option<&str>) -> LegacyResult<BTreeMap<String, String>> {
        let result = self.systemctl(&format!("--no-pager show \"{unit}\""), user)?;
        if result.status != 0 {
            let under_user = match user {
                Some(user) => format!(" under user \"{user}\""),
                None => String::new(),
            };
            return Err(LegacyError::Assertion(format!(
                "retrieving systemctl info for unit \"{unit}\"{under_user} failed with exit code {}",
                result.status
            )));
        }
        Ok(result
            .output
            .lines()
            .filter_map(|line| line.split_once('='))
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect())
    }

    /// Run a `systemctl` query, in the guest's system manager or in a user's.
    fn systemctl(&mut self, query: &str, user: Option<&str>) -> LegacyResult<CommandResult> {
        let Some(user) = user else {
            return self.execute(&format!("systemctl {query}"), Some(EXECUTE_DEFAULT_TIMEOUT));
        };
        // The user's query is built by the guest's own shell: `su -l` gives
        // the user a login environment, and the quoting keeps a query
        // carrying an apostrophe from ending the `$'…'` string early.
        let query = query.replace('\'', "\\'");
        self.execute(
            &format!(
                "su -l {user} --shell /bin/sh -c $'XDG_RUNTIME_DIR=/run/user/`id -u` systemctl --user {query}'"
            ),
            Some(EXECUTE_DEFAULT_TIMEOUT),
        )
    }

    /// Retry an attempt until it is ready, the bound is spent, or it refuses.
    ///
    /// The bound is spent by the attempts and the intervals between them, and
    /// one last attempt is made after it - the driver's arrangement, and the
    /// reason a wait that is about to succeed at its bound succeeds rather
    /// than failing on an attempt that was never made.
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    fn retry(
        &mut self,
        bound: Duration,
        mut attempt: impl FnMut(&mut Self) -> LegacyResult<bool>,
    ) -> LegacyResult<()> {
        let start = Instant::now();
        while start.elapsed() < bound {
            if attempt(self)? {
                return Ok(());
            }
            thread::sleep(RETRY_INTERVAL);
        }
        let elapsed = start.elapsed().as_secs_f64();
        if !attempt(self)? {
            return Err(LegacyError::Assertion(format!(
                "action timed out after {elapsed:.2} seconds (timeout={})",
                bound.as_secs()
            )));
        }
        Ok(())
    }

    /// Report one line the way the driver reported one, into the lane's
    /// report and into this check's own record of what it was doing.
    fn note(&mut self, line: &str) {
        emit_error(&format!("machine: {line}"));
        self.notes.push_str(line);
        self.notes.push('\n');
    }

    /// Report one diagnostics line, into the lane's report and into this
    /// check's own record.
    ///
    /// The prelude's lines went to the check's own stdout, which the lane
    /// reports as it arrives and files under that check's result; these go to
    /// the same two places under the same wording, so a reader of a ported
    /// check's failure reads what the fixture's failure printed.
    fn announce(&mut self, line: &str) {
        report(line);
        self.notes.push_str(line);
        self.notes.push('\n');
    }

    /// Start a check's diagnostics: the stage and the clock its lines are
    /// timed against are the check's own, the way the prelude's are the
    /// script's.
    fn begin_check(&mut self) {
        self.diagnostics = Diagnostics::new();
    }

    /// Report everything that explains a diagnostic wait that did not finish:
    /// the stage it was in, the rows it was asserting on, the journal lines
    /// that explain them, and the zone's composed explanation.
    ///
    /// Diagnostics only, and in the prelude's own order: the failing stage
    /// first, then each row's dump, then each explanation's journal, then the
    /// zone - so the reader of a failed lane has the row set before the lines
    /// that explain it. None of it can refuse: a dump that fails is reported
    /// as a failed diagnostic, which is why the wait's own error is the one
    /// that travels.
    fn explain_failure(
        &mut self,
        stage: &str,
        failing_wait: Option<&str>,
        rows: &[DiagRow<'_>],
        explain: &[DiagRow<'_>],
        error: &LegacyError,
    ) {
        let labels = rows
            .iter()
            .map(|(label, _)| *label)
            .collect::<Vec<_>>()
            .join(", ");
        let labels = if labels.is_empty() {
            "none".to_owned()
        } else {
            labels
        };
        let failing = if failing_wait.is_some() {
            format!(" wait={stage}")
        } else {
            String::new()
        };
        let head = format!(
            "[d2b] FAIL stage={stage} t={}{failing} rows=[{labels}]: {error}",
            self.diagnostics.elapsed(),
        );
        self.announce(&head);
        if let Some(command) = failing_wait {
            self.announce(&format!("[d2b] failing wait: {command}"));
        }
        for (label, dump) in rows.iter().copied() {
            self.diag(dump, &format!("row dump: {label}"));
        }
        for (unit, token) in explain.iter().copied() {
            self.diag(
                &journal_command(unit, token),
                &journal_label(unit, token),
            );
        }
        self.diag(&zone_explanation_command(), "zone explanation");
    }

    /// The closing line of a logged operation, timed as the driver timed it
    /// and reported only when the operation did not refuse.
    fn finished(&mut self, message: &str, started: Instant) {
        self.note(&format!(
            "(finished: {message}, in {:.2} seconds)",
            started.elapsed().as_secs_f64()
        ));
    }

    /// What the surface reported while a check ran, and then stops recording
    /// it: the next check's record is its own.
    fn take_notes(&mut self) -> String {
        std::mem::take(&mut self.notes)
    }

    /// Answer one request from a check's script.
    fn dispatch(&mut self, request: &Request) -> Reply {
        let answered = self.answer(request);
        match answered {
            Ok(value) => Reply::succeeded(value),
            Err(error) => match error {
                LegacyError::Assertion(message) => Reply::refused(&message),
                LegacyError::Guest(error) => Reply::unreachable(&error.to_string()),
            },
        }
    }

    /// Carry out one request, or say why it could not be carried out.
    fn answer(&mut self, request: &Request) -> LegacyResult<Value> {
        let timeout = request.seconds("timeout");
        match request.op.as_str() {
            "execute" => {
                let command = request.text(0, "command")?;
                let result = self.execute(&command, timeout)?;
                // The driver returned a sentinel status rather than reading
                // the block at all for a caller that asked not to. The block
                // is read here anyway: a command whose output is left on the
                // console is the next command's first line, and a check that
                // passes `check_output=False` would be reading it.
                if !request.boolean("check_output", true)? {
                    return Ok(json!([-2, ""]));
                }
                if !request.boolean("check_return", true)? {
                    return Ok(json!([-1, result.output]));
                }
                Ok(json!([result.status, result.output]))
            }
            "succeed" => {
                let commands = request.commands()?;
                let borrowed: Vec<&str> = commands.iter().map(String::as_str).collect();
                Ok(Value::String(self.succeed(&borrowed, timeout)?))
            }
            "fail" => {
                let commands = request.commands()?;
                let borrowed: Vec<&str> = commands.iter().map(String::as_str).collect();
                Ok(Value::String(self.fail(&borrowed, timeout)?))
            }
            "wait_until_succeeds" => Ok(Value::String(
                self.wait_until_succeeds(&request.text(0, "command")?, request.bound("timeout"))?,
            )),
            "wait_for_file" => {
                self.wait_for_file(&request.text(0, "filename")?, request.bound("timeout"))?;
                Ok(Value::Null)
            }
            "wait_for_unit" => {
                self.wait_for_unit(
                    &request.text(0, "unit")?,
                    request.optional_text("user").as_deref(),
                    request.bound("timeout"),
                )?;
                Ok(Value::Null)
            }
            "sleep" => {
                self.sleep(request.number(0, "secs")?)?;
                Ok(Value::Null)
            }
            other => Err(LegacyError::Guest(HarnessError::Configuration(format!(
                "a check asked the lane's guest-control surface for {other:?}, which is not one of the operations it re-provides"
            )))),
        }
    }
}

/// The journal dump one explanation prints, in the prelude's own words and
/// bounds.
///
/// An empty unit is the whole journal; an empty token is no filter, and the
/// filter is a fixed-string match because a token is a token, not a pattern.
fn journal_command(unit: &str, token: &str) -> String {
    let scope = if unit.is_empty() {
        String::new()
    } else {
        format!("-u {unit} ")
    };
    let select = if token.is_empty() {
        String::new()
    } else {
        format!("| grep -F -- '{token}' ")
    };
    format!(
        "journalctl {scope}--no-pager -o cat -b -n 4000 2>/dev/null {select}| tail -n 60 || true"
    )
}

/// What a journal dump is reported under.
fn journal_label(unit: &str, token: &str) -> String {
    let mut label = format!("journal {}", if unit.is_empty() { "all" } else { unit });
    if !token.is_empty() {
        label.push_str(&format!(" lines matching '{token}'"));
    }
    label
}

/// The composed `d2b debug` report the prelude prints for a failing stage:
/// the zone's ownership tree, the row that is not settled, and the structured
/// failure behind it.
///
/// Non-fatal by construction - the command ends in `|| true` and is bounded -
/// so a failure that happened before the daemon was reachable prints its own
/// stage rather than a diagnostic error.
fn zone_explanation_command() -> String {
    format!(
        "runuser -u {DIAG_USER} -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock \
         timeout 60 d2b --zone {DIAG_ZONE} debug {DIAG_ZONE} 2>&1 || true"
    )
}

/// A guest a check's script can be run against, and the surface that runs it.
pub struct LegacyGuest {
    control: GuestControl,
    work_dir: PathBuf,
}

impl LegacyGuest {
    /// Attach the guest-control console to a booted guest and wait for its
    /// shell.
    pub fn attach(guest: &mut ActiveGuest) -> Result<Self> {
        let console = Console::attach(guest)?;
        let work_dir = console.work_dir.clone();
        Ok(Self {
            control: GuestControl::new(console),
            work_dir,
        })
    }

    /// Run one unported check's script against this guest.
    ///
    /// The check's script is executed by an interpreter rather than being
    /// interpreted by the lane, because the assertions in it are the
    /// assertions the check has always made and rewriting them is the port
    /// this whole transition is built to make one check at a time. What the
    /// script calls is this module: every operation it performs is a request
    /// that arrives here and is carried out against the guest, and the log
    /// lines around those operations are written as they happen.
    pub fn run(&mut self, check: &LegacyCheck) -> Result<LegacyOutcome> {
        let script = self.work_dir.join("legacy-check.py");
        let control = self.work_dir.join("legacy-control.sock");
        fs::write(&script, &check.script)
            .map_err(|error| HarnessError::io(format!("writing {}", script.display()), error))?;
        if control.exists() {
            fs::remove_file(&control)
                .map_err(|error| HarnessError::io(format!("removing {}", control.display()), error))?;
        }
        let listener = UnixListener::bind(&control)
            .map_err(|error| HarnessError::io(format!("binding {}", control.display()), error))?;
        let interpreter = interpreter()?;
        let mut child = spawn_check(&interpreter, &control, &script)?;
        let (served, out, err) = thread::scope(|scope| {
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            let out = scope.spawn(move || stdout.map_or_else(String::new, |pipe| pump(pipe, false)));
            let err = scope.spawn(move || stderr.map_or_else(String::new, |pipe| pump(pipe, true)));
            let served = serve(&listener, &mut self.control, &mut child);
            let out = out.join().unwrap_or_default();
            let err = err.join().unwrap_or_default();
            (served, out, err)
        });
        let status = child
            .wait()
            .map_err(|error| HarnessError::io("waiting for the check's script", error))?;
        served?;
        let mut detail = out;
        detail.push_str(&err);
        detail.push_str(&self.control.take_notes());
        Ok(LegacyOutcome {
            name: check.name.clone(),
            passed: status.success(),
            detail,
        })
    }

    /// Run one ported check's assertions against this guest.
    ///
    /// A ported check's assertions are the lane's own Rust: the same
    /// operations, in the same order, with the same bounds its fixture made,
    /// so there is no interpreter between an assertion and the guest it
    /// asserts against. What is reported is what [`Self::run`] reports for a
    /// script - the surface's own log lines, and the check's failure - which
    /// is what makes a check's diagnostics readable the same way before and
    /// after its port.
    pub fn run_ported(
        &mut self,
        name: &str,
        assertions: Assertions,
    ) -> Result<LegacyOutcome> {
        self.control.begin_check();
        let passed = match assertions(&mut self.control) {
            Ok(()) => true,
            Err(error) => {
                self.control.note(&format!("check failed: {error}"));
                false
            }
        };
        Ok(LegacyOutcome {
            name: name.to_owned(),
            passed,
            detail: self.control.take_notes(),
        })
    }

    /// Take the console back after the guest was restarted onto a restored
    /// disk.
    ///
    /// The channel is one connection for the life of the emulator process, so
    /// the bytes the previous guest wrote as it shut down are still in it when
    /// the new guest's shell greets. Reading a command's output from that
    /// position decodes the old guest's leftovers as the new one's answer -
    /// which reads as a command that returned nonsense, not as a console that
    /// needed resynchronising. Waiting for the greeting again is what puts
    /// the reader back on a command boundary.
    pub fn resync(&mut self) -> Result<()> {
        self.control.console.resync(SHELL_GREETING_TIMEOUT)
    }

    /// The working directory this guest's check scripts are written to.
    pub fn work_dir(&self) -> &Path {
        &self.work_dir
    }
}

/// One request from a check's script.
#[derive(Debug, Deserialize)]
struct Request {
    op: String,
    #[serde(default)]
    args: Vec<Value>,
    #[serde(default)]
    kwargs: BTreeMap<String, Value>,
}

impl Request {
    /// The commands of a `succeed` or a `fail`, which take any number of
    /// them and concatenate the outputs.
    fn commands(&self) -> LegacyResult<Vec<String>> {
        self.args
            .iter()
            .map(|argument| {
                argument.as_str().map(str::to_owned).ok_or_else(|| {
                    LegacyError::Guest(HarnessError::Configuration(format!(
                        "`{}` was given an argument that is not a command",
                        self.op
                    )))
                })
            })
            .collect()
    }

    /// A required string argument.
    fn text(&self, index: usize, name: &str) -> LegacyResult<String> {
        self.args
            .get(index)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| self.malformed(name))
    }

    /// An optional string argument.
    fn optional_text(&self, name: &str) -> Option<String> {
        self.kwargs
            .get(name)
            .and_then(Value::as_str)
            .map(str::to_owned)
    }

    /// A required number argument.
    fn number(&self, index: usize, name: &str) -> LegacyResult<u64> {
        self.args
            .get(index)
            .and_then(Value::as_u64)
            .ok_or_else(|| self.malformed(name))
    }

    /// A required boolean argument, with the driver's own default.
    fn boolean(&self, name: &str, default: bool) -> LegacyResult<bool> {
        match self.kwargs.get(name) {
            None => Ok(default),
            Some(Value::Bool(value)) => Ok(*value),
            Some(_) => Err(self.malformed(name)),
        }
    }

    /// A bound in seconds, where the absence of one means unbounded.
    fn seconds(&self, name: &str) -> Option<u64> {
        self.kwargs
            .get(name)
            .and_then(Value::as_u64)
    }

    /// A bound as a duration, defaulting the way the driver defaulted it.
    fn bound(&self, name: &str) -> Duration {
        Duration::from_secs(self.seconds(name).unwrap_or(EXECUTE_DEFAULT_TIMEOUT))
    }

    fn malformed(&self, name: &str) -> LegacyError {
        LegacyError::Guest(HarnessError::Configuration(format!(
            "a check called `{}` without a usable {name}",
            self.op
        )))
    }
}

/// One answer to a check's script.
#[derive(Debug, Serialize)]
struct Reply {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    /// Whether the failure was the check's own verdict rather than the
    /// lane's, so the script reports it as the assertion it is.
    assertion: bool,
}

impl Reply {
    fn succeeded(value: Value) -> Self {
        Self {
            ok: true,
            value: Some(value),
            error: None,
            assertion: false,
        }
    }

    fn refused(message: &str) -> Self {
        Self {
            ok: false,
            value: None,
            error: Some(message.to_owned()),
            assertion: true,
        }
    }

    fn unreachable(message: &str) -> Self {
        Self {
            ok: false,
            value: None,
            error: Some(message.to_owned()),
            assertion: false,
        }
    }
}

/// Run the check's interpreter, with the bridge and the script it runs.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn spawn_check(interpreter: &Path, control: &Path, script: &Path) -> Result<Child> {
    let mut command = Command::new(interpreter);
    command
        // Unbuffered, so the check's own output reaches the lane's report as
        // it is printed rather than when the interpreter exits.
        .arg("-u")
        .arg("-c")
        .arg(BRIDGE)
        .arg(control)
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.spawn().map_err(|error| HarnessError::Spawn {
        detail: format!("{}: {error}", interpreter.display()),
    })
}

/// Answer a check's requests until its script is finished with them.
fn serve(
    listener: &UnixListener,
    control: &mut GuestControl,
    check: &mut Child,
) -> Result<()> {
    let stream = accept(listener, Some(check))?;
    let reader = io::BufReader::new(
        stream
            .try_clone()
            .map_err(|error| HarnessError::io("cloning the control socket", error))?,
    );
    let mut writer = stream;
    for line in reader.lines() {
        let line = line
            .map_err(|error| HarnessError::io("reading a check's request", error))?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Request>(&line) {
            Ok(request) => control.dispatch(&request),
            Err(error) => Reply::unreachable(&format!("a check sent a request the surface cannot read: {error}")),
        };
        let mut answer = serde_json::to_string(&reply).map_err(|error| {
            HarnessError::Configuration(format!("the surface could not answer a request: {error}"))
        })?;
        answer.push('\n');
        writer
            .write_all(answer.as_bytes())
            .map_err(|error| HarnessError::io("answering a check's request", error))?;
        writer
            .flush()
            .map_err(|error| HarnessError::io("answering a check's request", error))?;
    }
    Ok(())
}

/// Read one of the check's streams to its end, reporting it as it arrives.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn pump(pipe: impl io::Read, errors: bool) -> String {
    let mut captured = String::new();
    for line in io::BufReader::new(pipe).lines().map_while(std::result::Result::ok) {
        if errors {
            emit_error(&line);
        } else {
            report(&line);
        }
        captured.push_str(&line);
        captured.push('\n');
    }
    captured
}

/// Write one line to the lane's report.
fn emit_error(line: &str) {
    let mut stderr = io::stderr().lock();
    let _ = writeln!(stderr, "{line}");
    let _ = stderr.flush();
}

/// The interpreter a check's script runs under.
///
/// A lane that pins one names it, which is the hermetic answer and the one
/// the lane's test target uses: the interpreter is a declared runfile rather
/// than whatever `python3` a developer's shell happens to resolve. Without
/// that, the runfiles tree is searched for the declared interpreter, and a
/// contributor running the harness outside Bazel falls back to `PATH`.
fn interpreter() -> Result<PathBuf> {
    if let Some(named) = env::var_os(PYTHON) {
        return Ok(PathBuf::from(named));
    }
    for root in [env::var_os("RUNFILES_DIR"), env::var_os("TEST_SRCDIR")]
        .into_iter()
        .flatten()
    {
        for candidate in ["python3/bin/python3", "python3+/bin/python3"] {
            let path = PathBuf::from(&root).join(candidate);
            if path.is_file() {
                return Ok(path);
            }
        }
    }
    Ok(PathBuf::from("python3"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A guest that answers the console from a fixed script, and records
    /// what it was asked.
    ///
    /// It speaks the same wire form the guest's shell does, so a test
    /// exercises the real protocol rather than a stand-in for it: the
    /// command the surface sent, a base64 block, the status request, the
    /// status.
    fn guest(stream: UnixStream, answers: Vec<(i32, String)>) -> (Vec<String>, Vec<String>) {
        let mut reader = io::BufReader::new(
            stream
                .try_clone()
                .expect("a console socket can be cloned for reading"),
        );
        let mut writer = stream;
        let mut asked = Vec::new();
        let mut statuses = Vec::new();
        for (status, output) in answers {
            let mut line = String::new();
            reader.read_line(&mut line).expect("the command line");
            let _ = writer.write_all(format!("{output}\n").as_bytes());
            let mut request = String::new();
            reader.read_line(&mut request).expect("the status request");
            let _ = writer.write_all(format!("{status}\n").as_bytes());
            asked.push(line);
            statuses.push(request);
        }
        (asked, statuses)
    }

    /// One answer's worth of base64, the way the guest's shell frames it.
    fn block(output: &str) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut encoded = String::new();
        for chunk in output.as_bytes().chunks(3) {
            let group = [
                chunk[0],
                chunk.get(1).copied().unwrap_or(0),
                chunk.get(2).copied().unwrap_or(0),
            ];
            let packed =
                (u32::from(group[0]) << 16) | (u32::from(group[1]) << 8) | u32::from(group[2]);
            // Three bytes are four characters, two are three, and one is two;
            // whatever is left of the group is padding.
            let characters = chunk.len() * 8 / 6 + 1;
            for (index, shift) in [18, 12, 6, 0].into_iter().enumerate() {
                let digit = ((packed >> shift) & 0x3f) as usize;
                encoded.push(if index < characters {
                    ALPHABET[digit] as char
                } else {
                    '='
                });
            }
        }
        encoded
    }

    /// Run one operation against a scripted guest, and hand back what the
    /// surface reported and what the guest was asked.
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    fn against(
        answers: Vec<(i32, String)>,
        body: impl FnOnce(&mut GuestControl) -> LegacyResult<()>,
    ) -> (LegacyResult<()>, String, Vec<String>, Vec<String>) {
        let (surface, console) = UnixStream::pair().expect("a console socket pair");
        thread::scope(|scope| {
            let asked = scope.spawn(move || guest(console, answers));
            let mut control = GuestControl::new(Console::serving(surface, PathBuf::from("/dev/null")));
            let outcome = body(&mut control);
            let notes = std::mem::take(&mut control.notes);
            let (asked, statuses) = asked.join().expect("the scripted guest");
            (outcome, notes, asked, statuses)
        })
    }

    /// The refusal a check's script would see, if the surface refused.
    fn refused(outcome: &LegacyResult<()>) -> String {
        match outcome {
            Err(LegacyError::Assertion(message)) => message.clone(),
            other => panic!("expected a refused assertion, got {other:?}"),
        }
    }

    #[test]
    fn a_command_reaches_the_guest_in_the_drivers_wire_form() {
        let (outcome, notes, asked, statuses) = against(vec![(0, block("hi\n"))], |control| {
            let result = control.execute("echo hi", Some(5))?;
            assert_eq!(result.status, 0);
            assert_eq!(result.output, "hi\n");
            Ok(())
        });
        assert!(outcome.is_ok(), "{outcome:?}");
        assert_eq!(asked.len(), 1);
        assert_eq!(
            asked[0],
            "timeout 5 bash -c 'set -euo pipefail; echo hi' | (base64 -w 0; echo)\n"
        );
        assert_eq!(statuses, vec!["echo ${PIPESTATUS[0]}\n".to_owned()]);
        assert!(notes.is_empty(), "{notes}");
    }

    #[test]
    fn an_unbounded_command_carries_no_bound() {
        let (_, _, asked, _) = against(vec![(0, block(""))], |control| {
            control.execute("true", None)?;
            Ok(())
        });
        assert_eq!(
            asked[0],
            "bash -c 'set -euo pipefail; true' | (base64 -w 0; echo)\n"
        );
    }

    #[test]
    fn a_command_carrying_a_quote_still_reaches_the_guest_intact() {
        let (_, _, asked, _) = against(vec![(0, block(""))], |control| {
            control.execute("sh -c 'echo it'\"'\"'s'", None)?;
            Ok(())
        });
        assert_eq!(
            asked[0],
            "bash -c 'set -euo pipefail; sh -c '\"'\"'echo it'\"'\"'\"'\"'\"'\"'\"'\"'s'\"'\"'' | (base64 -w 0; echo)\n"
        );
    }

    #[test]
    fn a_refused_command_reports_the_drivers_message_and_its_output() {
        let (outcome, notes, _, _) = against(vec![(3, block("no such thing\n"))], |control| {
            control.succeed(&["test -e /run/d2b/public.sock"], None)?;
            Ok(())
        });
        assert_eq!(
            refused(&outcome),
            "command `test -e /run/d2b/public.sock` failed (exit code 3)"
        );
        assert!(notes.contains("must succeed: test -e /run/d2b/public.sock\n"), "{notes}");
        assert!(notes.contains("output: no such thing\n"), "{notes}");
    }

    #[test]
    fn a_command_that_succeeds_returns_its_output_and_logs_its_finish() {
        let (outcome, notes, _, _) = against(vec![(0, block("d2bd.service\n"))], |control| {
            let output = control.succeed(&["systemctl is-active d2bd.service"], None)?;
            assert_eq!(output, "d2bd.service\n");
            Ok(())
        });
        assert!(outcome.is_ok(), "{outcome:?}");
        assert!(notes.contains("(finished: must succeed: systemctl is-active d2bd.service, in "), "{notes}");
    }

    #[test]
    fn several_commands_concatenate_their_outputs() {
        let (outcome, _, _, _) = against(
            vec![(0, block("one\n")), (0, block("two\n"))],
            |control| {
                let output = control.succeed(&["echo one", "echo two"], None)?;
                assert_eq!(output, "one\ntwo\n");
                Ok(())
            },
        );
        assert!(outcome.is_ok(), "{outcome:?}");
    }

    #[test]
    fn a_command_that_was_meant_to_fail_is_refused_when_it_succeeds() {
        let (outcome, notes, _, _) = against(vec![(0, block(""))], |control| {
            control.fail(&["test -e /nope"], None)?;
            Ok(())
        });
        assert_eq!(
            refused(&outcome),
            "command `test -e /nope` unexpectedly succeeded"
        );
        assert!(notes.contains("must fail: test -e /nope\n"), "{notes}");
    }

    #[test]
    fn a_command_meant_to_fail_returns_its_output_when_it_does() {
        let (outcome, _, _, _) = against(vec![(1, block("refused\n"))], |control| {
            let output = control.fail(&["d2b list Zone"], None)?;
            assert_eq!(output, "refused\n");
            Ok(())
        });
        assert!(outcome.is_ok(), "{outcome:?}");
    }

    #[test]
    fn a_wait_that_never_succeeds_reports_its_bound_and_the_last_output() {
        let (outcome, notes, asked, _) =
            against(vec![(1, block("still starting\n")), (1, block("still starting\n"))], |control| {
                control.wait_until_succeeds("systemctl is-active d2bd.service", Duration::from_secs(1))?;
                Ok(())
            });
        let message = refused(&outcome);
        assert!(message.starts_with("action timed out after "), "{message}");
        assert!(message.ends_with(" seconds (timeout=1)"), "{message}");
        assert_eq!(asked.len(), 2, "one attempt in the loop, one after the bound");
        assert!(notes.contains("output: still starting"), "{notes}");
        assert!(notes.contains("waiting for success: systemctl is-active d2bd.service\n"), "{notes}");
    }

    #[test]
    fn a_wait_that_succeeds_returns_the_output_of_the_attempt_that_did() {
        let (outcome, _, _, _) = against(
            vec![(1, block("not yet\n")), (0, block("active\n"))],
            |control| {
                let output = control.wait_until_succeeds("systemctl is-active d2bd.service", Duration::from_secs(2));
                assert_eq!(output.unwrap(), "active\n");
                Ok(())
            },
        );
        assert!(outcome.is_ok(), "{outcome:?}");
    }

    #[test]
    fn a_file_that_never_appears_reports_after_its_bound() {
        let (outcome, notes, asked, _) = against(vec![(1, block("")), (1, block(""))], |control| {
            control.wait_for_file("/run/d2b/public.sock", Duration::from_secs(1))?;
            Ok(())
        });
        let message = refused(&outcome);
        assert!(message.ends_with(" seconds (timeout=1)"), "{message}");
        assert_eq!(asked[0], "timeout 900 bash -c 'set -euo pipefail; test -e /run/d2b/public.sock' | (base64 -w 0; echo)\n");
        assert!(notes.contains("waiting for file '/run/d2b/public.sock'\n"), "{notes}");
    }

    #[test]
    fn a_unit_that_failed_ends_the_wait_at_once() {
        let (outcome, _, asked, _) = against(vec![(0, block("ActiveState=failed\n"))], |control| {
            control.wait_for_unit("d2bd.service", None, Duration::from_secs(30))?;
            Ok(())
        });
        assert_eq!(refused(&outcome), "unit \"d2bd.service\" reached state \"failed\"");
        assert_eq!(asked.len(), 1, "the state is read once, and refused on it");
        assert_eq!(
            asked[0],
            "timeout 900 bash -c 'set -euo pipefail; systemctl --no-pager show \"d2bd.service\" --property=\"ActiveState\"' | (base64 -w 0; echo)\n"
        );
    }

    #[test]
    fn a_unit_that_is_inactive_with_nothing_pending_ends_the_wait() {
        let (outcome, _, asked, _) = against(
            vec![
                (0, block("ActiveState=inactive\n")),
                (0, block("No jobs to be processed.\n")),
                (0, block("ActiveState=inactive\nSubState=dead\n")),
            ],
            |control| {
                control.wait_for_unit("d2b-broker.socket", None, Duration::from_secs(30))?;
                Ok(())
            },
        );
        assert_eq!(
            refused(&outcome),
            "unit \"d2b-broker.socket\" is inactive and there are no pending jobs"
        );
        assert_eq!(asked.len(), 3, "the state, the job list, and the unit's own state");
    }

    #[test]
    fn a_unit_that_is_active_satisfies_the_wait() {
        let (outcome, notes, _, _) = against(vec![(0, block("ActiveState=active\n"))], |control| {
            control.wait_for_unit("multi-user.target", None, Duration::from_secs(30))?;
            Ok(())
        });
        assert!(outcome.is_ok(), "{outcome:?}");
        assert!(notes.contains("waiting for unit multi-user.target\n"), "{notes}");
    }

    #[test]
    fn a_units_state_is_read_in_the_users_own_manager() {
        let (_, _, asked, _) = against(vec![(0, block("ActiveState=active\n"))], |control| {
            control.wait_for_unit("graphical-session.target", Some("alice"), Duration::from_secs(30))?;
            Ok(())
        });
        assert!(asked[0].contains("su -l alice --shell /bin/sh -c"), "{asked:?}");
        assert!(
            asked[0].contains("systemctl --user --no-pager show"),
            "{asked:?}"
        );
    }

    #[test]
    fn a_guest_that_never_announces_its_shell_is_reported_by_what_it_should_have_said() {
        let (surface, console) = UnixStream::pair().expect("a console socket pair");
        let mut writer = console;
        let _ = writer.write_all(b"connecting to host...\n");
        let mut control = GuestControl::new(Console::serving(surface, PathBuf::from("/dev/null")));
        let error = control
            .console
            .await_shell(Duration::from_millis(50))
            .expect_err("a console that never greets is not a usable one");
        let message = error.to_string();
        assert!(message.contains(SHELL_GREETING), "{message}");
        assert!(message.contains("virtio serial console"), "{message}");
    }

    #[test]
    fn a_guest_console_that_closes_mid_command_is_reported() {
        let (surface, console) = UnixStream::pair().expect("a console socket pair");
        let mut control = GuestControl::new(Console::serving(surface, PathBuf::from("/dev/null")));
        drop(console);
        let error = control
            .console
            .run("true", None)
            .expect_err("a console that closed cannot answer");
        assert!(error.to_string().contains("console"), "{error}");
    }

    #[test]
    fn a_refused_assertion_travels_to_the_check_as_an_assertion() {
        let (surface, console) = UnixStream::pair().expect("a console socket pair");
        thread::scope(|scope| {
            let _asked = scope.spawn(move || guest(console, vec![(1, block(""))]));
            let mut control = GuestControl::new(Console::serving(surface, PathBuf::from("/dev/null")));
            let request = request("succeed", json!(["test -e /nope"]));
            let reply = control.dispatch(&request);
            assert!(!reply.ok);
            assert!(reply.assertion, "a refused command is the check's own verdict");
            assert_eq!(reply.error.as_deref(), Some("command `test -e /nope` failed (exit code 1)"));
        });
    }

    #[test]
    fn an_operation_the_surface_does_not_carry_is_named_rather_than_ignored() {
        let (surface, _console) = UnixStream::pair().expect("a console socket pair");
        let mut control = GuestControl::new(Console::serving(surface, PathBuf::from("/dev/null")));
        let reply = control.dispatch(&request("wait_for_open_port", json!(["22"])));
        assert!(!reply.ok);
        assert!(!reply.assertion, "a missing operation is the lane's, not the check's");
        assert!(
            reply
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("wait_for_open_port"),
            "{reply:?}"
        );
    }

    #[test]
    fn a_command_whose_output_was_not_wanted_does_not_desync_the_console() {
        let (surface, console) = UnixStream::pair().expect("a console socket pair");
        thread::scope(|scope| {
            let _asked = scope.spawn(move || {
                guest(
                    console,
                    vec![(0, block("first\n")), (0, block("second\n"))],
                )
            });
            let mut control = GuestControl::new(Console::serving(surface, PathBuf::from("/dev/null")));
            let mut kwargs = BTreeMap::new();
            kwargs.insert("check_output".to_owned(), Value::Bool(false));
            let mut quiet = request("execute", json!(["systemctl restart d2bd.service"]));
            quiet.kwargs = kwargs;
            let quiet = control.dispatch(&quiet);
            assert_eq!(quiet.value, Some(json!([-2, ""])));
            let loud = control.dispatch(&request("execute", json!(["echo second"])));
            assert_eq!(
                loud.value,
                Some(json!([0, "second\n".to_owned()])),
                "the second command reads its own output, not the first one's"
            );
        });
    }

    #[test]
    fn a_sleep_is_guest_time() {
        let (_, _, asked, _) = against(vec![(0, block(""))], |control| {
            control.sleep(5)?;
            Ok(())
        });
        assert_eq!(
            asked[0],
            "bash -c 'set -euo pipefail; sleep 5' | (base64 -w 0; echo)\n"
        );
    }

    /// One request, as a check's script sends it.
    fn request(op: &str, args: Value) -> Request {
        serde_json::from_str(&json!({ "op": op, "args": args }).to_string()).expect("a request")
    }

    #[test]
    fn a_string_is_quoted_the_way_the_guests_shell_needs_it_quoted() {
        // The cases are the interpreter's own: an unreserved ASCII string
        // passes through, an empty one is two quotes, and anything else is
        // single-quoted with an embedded quote closed, double-quoted, and
        // reopened. A different rule would change what the guest runs for a
        // command carrying a quote.
        assert_eq!(shlex_quote(""), "''");
        assert_eq!(shlex_quote("abc"), "abc");
        assert_eq!(
            shlex_quote("test -e /run/d2b/public.sock"),
            "'test -e /run/d2b/public.sock'"
        );
        assert_eq!(shlex_quote("set -euo pipefail; echo hi"), "'set -euo pipefail; echo hi'");
        assert_eq!(shlex_quote("a'b"), "'a'\"'\"'b'");
        assert_eq!(shlex_quote("a\"b"), "'a\"b'");
        assert_eq!(shlex_quote("x$y"), "'x$y'");
        assert_eq!(shlex_quote("a b\nc"), "'a b\nc'");
        assert_eq!(shlex_quote("a+b,c-d.e/f:g=h@i%j_k"), "a+b,c-d.e/f:g=h@i%j_k");
    }

    #[test]
    fn a_guests_output_block_decodes_to_what_the_command_wrote() {
        assert_eq!(base64_decode("").expect("an empty block"), Vec::<u8>::new());
        assert_eq!(base64_decode("aGk=").expect("a block"), b"hi".to_vec());
        assert_eq!(base64_decode("aGVsbG8=").expect("a block"), b"hello".to_vec());
        assert_eq!(
            base64_decode("YWJjZGVmZ2hpamtsbW5vcHFyc3R1dnd4eXo=").expect("a block"),
            b"abcdefghijklmnopqrstuvwxyz".to_vec()
        );
        // A block long enough to overflow the accumulator, and one that
        // arrives wrapped the way a guest's shell wraps it.
        let long = "eHh4".repeat(250);
        assert_eq!(base64_decode(&long).expect("a long block").len(), 750);
        assert_eq!(base64_decode("aGk=\n").expect("a wrapped block"), b"hi".to_vec());
        assert!(base64_decode("not base64!").is_err());
    }
}
