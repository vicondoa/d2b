//! Detached exec: the transient-unit abstraction and its production
//! `systemd-run`/`systemctl` implementation.
//!
//! The full detached registry (slot allocator, quota accounting, creation
//! state machine, re-adoption, TTL/GC, live reconciliation) is built on top of
//! this trait. Only the abstraction + production manager shape live here so the
//! registry can be unit-tested against an in-memory fake.

use std::ffi::OsString;
use std::fmt;
use std::path::Path;
use std::path::PathBuf;

use async_trait::async_trait;

use nixling_exec_runner::paths::RUN_DIR;

/// Redacted, typed transient-unit failure. Carries no command output or paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitError {
    /// The helper subprocess could not be spawned (binary missing, fork
    /// failure, ...).
    SpawnFailed,
    /// The helper subprocess returned a non-zero status.
    NonZeroExit,
    /// The helper did not complete within the bounded window.
    Timeout,
    /// Detached units are not configured for this guest (no runtime config).
    Unsupported,
    /// Anything else, with no payload surfaced to callers.
    Internal,
}

/// A transient unit the manager currently knows about (re-adoption input).
#[derive(Clone, PartialEq, Eq)]
pub struct ManagedUnit {
    pub slot: u32,
    pub kind: ManagedUnitKind,
    /// True when the unit is loaded and active/activating.
    pub active: bool,
    /// The unit's identity (`Slice` + `ExecStart`) as resolved by
    /// `systemctl show`, or [`UnitIdentity::Unqueried`] when that query could
    /// not be performed/parsed. The distinction is load-bearing: an ACTIVE
    /// unit whose identity is `Unqueried` classifies as `Unknown` (retry),
    /// never `Foreign` (destructive) — only a queried identity that actually
    /// mismatches is `Foreign`.
    pub identity: UnitIdentity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedUnitKind {
    Runner,
    Workload,
}

impl ManagedUnit {
    fn name(&self) -> String {
        match self.kind {
            ManagedUnitKind::Runner => unit_name(self.slot),
            ManagedUnitKind::Workload => workload_unit_name(self.slot),
        }
    }
}

/// The result of querying a unit's identity (`Slice` + `ExecStart`) via
/// `systemctl show`. A *queried-but-empty* identity (systemd reported empty
/// `Slice=`/`ExecStart=`) is a genuine mismatch, NOT a query failure; an
/// *unqueried* identity means the `show` step never produced a usable value
/// (spawn failed, non-zero exit, unparsable, or no block for this unit). Only
/// the queried case can drive a `Foreign` classification.
#[derive(Clone, PartialEq, Eq)]
pub enum UnitIdentity {
    /// `systemctl show` was read for this unit. `slice`/`exec_start` carry
    /// exactly what systemd reported (either may be `None` for an empty value).
    Queried {
        slice: Option<String>,
        exec_start: Option<String>,
        binds_to: Option<String>,
        part_of: Option<String>,
        after: Option<String>,
    },
    /// The identity query failed or produced no block for this unit; its
    /// identity is unknown.
    Unqueried,
}

/// The structural decomposition of a systemd-rendered `ExecStart` value
/// (`{ path=<exe> ; argv[]=<t0> <t1> ... ; ... }`): the resolved executable
/// path and the argv token sequence. Used for the structural identity check
/// so an impostor cannot pass by merely embedding the expected substrings in
/// an unrelated argument.
#[derive(Clone, PartialEq, Eq)]
pub struct ParsedExecStart {
    pub exe: String,
    pub argv: Vec<String>,
}

impl fmt::Debug for ManagedUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ManagedUnit")
            .field("slot", &self.slot)
            .field("kind", &self.kind)
            .field("active", &self.active)
            .field("identity", &self.identity)
            .finish()
    }
}

impl fmt::Debug for UnitIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Queried {
                slice,
                exec_start,
                binds_to,
                part_of,
                after,
            } => f
                .debug_struct("Queried")
                .field("slice", &slice.as_deref())
                .field("has_exec_start", &exec_start.is_some())
                .field("has_binds_to", &binds_to.is_some())
                .field("has_part_of", &part_of.is_some())
                .field("has_after", &after.is_some())
                .finish(),
            Self::Unqueried => f.write_str("Unqueried"),
        }
    }
}

impl fmt::Debug for ParsedExecStart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ParsedExecStart")
            .field("exe", &self.exe)
            .field("argv_len", &self.argv.len())
            .finish()
    }
}

/// Parse a systemd-rendered `ExecStart` value of the form
/// `{ path=<exe> ; argv[]=<t0> <t1> ... ; ignore_errors=... ; ... }` into its
/// executable path and argv token sequence. Fields are `;`-separated; argv
/// tokens are whitespace-separated. Returns `None` when the value lacks the
/// expected `path=`/`argv[]=` structure (so the caller treats it as a
/// mismatch, never a match).
pub fn parse_exec_start(value: &str) -> Option<ParsedExecStart> {
    let inner = value.trim().strip_prefix('{')?;
    let inner = inner.strip_suffix('}').unwrap_or(inner);
    let mut exe: Option<String> = None;
    let mut argv: Option<Vec<String>> = None;
    for field in inner.split(';') {
        let field = field.trim();
        if let Some(v) = field.strip_prefix("path=") {
            if exe.is_none() {
                exe = Some(v.trim().to_owned());
            }
        } else if let Some(v) = field.strip_prefix("argv[]=") {
            if argv.is_none() {
                argv = Some(parse_exec_argv(v.trim())?);
            }
        }
    }

    fn parse_exec_argv(value: &str) -> Option<Vec<String>> {
        let mut argv = Vec::new();
        let mut current = String::new();
        let mut quote: Option<char> = None;
        let mut escaped = false;
        let mut in_token = false;

        for ch in value.chars() {
            if escaped {
                current.push(ch);
                in_token = true;
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                in_token = true;
                continue;
            }
            if let Some(q) = quote {
                if ch == q {
                    quote = None;
                } else {
                    current.push(ch);
                }
                in_token = true;
                continue;
            }
            match ch {
                '\'' | '"' => {
                    quote = Some(ch);
                    in_token = true;
                }
                c if c.is_whitespace() => {
                    if in_token {
                        argv.push(std::mem::take(&mut current));
                        in_token = false;
                    }
                }
                _ => {
                    current.push(ch);
                    in_token = true;
                }
            }
        }
        if escaped || quote.is_some() {
            return None;
        }
        if in_token {
            argv.push(current);
        }
        Some(argv)
    }
    let exe = exe?;
    if exe.is_empty() {
        return None;
    }
    Some(ParsedExecStart {
        exe,
        argv: argv.unwrap_or_default(),
    })
}

/// Extract the RAW `path=` and `argv[]=` field values from a systemd-rendered
/// `ExecStart` (`{ path=<exe> ; argv[]=<args> ; ignore_errors=... ; ... }`)
/// WITHOUT tokenizing the argv.
///
/// `systemctl show -p ExecStart` renders `argv[]` as a literal single-space
/// join of the argument vector with NO escaping of embedded spaces, quotes, or
/// backslashes. The token parser (`parse_exec_start`) is therefore the wrong
/// tool for the workload-identity check: `validate_command` permits argv bytes
/// the tokenizer rejects (an unmatched `"`, a trailing `\`) or mis-splits (a
/// `;`, which `parse_exec_start` treats as a field delimiter — a fail-OPEN
/// truncation that would compare only a shared prefix). The caller compares
/// this raw `argv[]=` value byte-for-byte against the expected argv rendered the
/// same lossy way, so the match is symmetric, never tokenizes user bytes, and
/// cannot truncate at a user `;`.
///
/// The `argv[]=` value is delimited by the fixed ` ; ignore_errors=` field that
/// systemd always emits immediately after it, so a `;` inside a user argument
/// (e.g. `sh -c 'a ; b'`) is preserved rather than treated as a field break.
/// Returns `None` if either field is absent (treated as a mismatch, never a
/// match).
pub fn exec_start_raw_fields(value: &str) -> Option<(String, String)> {
    let path = {
        let after = value.split_once("path=")?.1;
        after.split_once(" ; ")?.0.trim().to_owned()
    };
    let argv = {
        let after = value.split_once("argv[]=")?.1;
        after.split_once(" ; ignore_errors=")?.0.trim().to_owned()
    };
    Some((path, argv))
}

/// Absolute, controlled paths the manager needs to launch a runner unit. All
/// values are host-supplied constants (never caller-derived).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerUnitPaths {
    /// Absolute path to the `nixling-exec-runner` binary.
    pub exec_runner_path: PathBuf,
    /// Base directory for slot dirs (production: `/run/nixling-exec`).
    pub run_dir: PathBuf,
}

impl RunnerUnitPaths {
    pub fn new(exec_runner_path: impl Into<PathBuf>) -> Self {
        Self {
            exec_runner_path: exec_runner_path.into(),
            run_dir: PathBuf::from(RUN_DIR),
        }
    }
}

/// Manages the per-slot transient units that host detached runners. Async and
/// `Send + Sync + 'static` (mirrors `ProcessSpawner`); held as
/// `Arc<dyn TransientUnitManager>`. Every method is idempotent and
/// non-blocking (subprocesses run on the tokio runtime).
#[async_trait]
pub trait TransientUnitManager: Send + Sync + 'static {
    /// Start `nixling-exec-<slot>.service`. Blocks (on the runtime) until the
    /// unit job is registered, so a successful return proves the unit exists.
    /// `ceiling_sec == 0` means no `RuntimeMaxSec` (indefinite runtime).
    async fn start_transient_unit(
        &self,
        slot: u32,
        ceiling_sec: u64,
        paths: &RunnerUnitPaths,
    ) -> Result<(), UnitError>;

    /// Stop the unit for `slot` (best-effort, idempotent).
    async fn stop_unit(&self, slot: u32) -> Result<(), UnitError>;

    /// Clear a failed unit for `slot` (best-effort, idempotent).
    async fn reset_failed(&self, slot: u32) -> Result<(), UnitError>;

    /// Enumerate the managed `nixling-exec-*` units currently known to systemd.
    async fn list_managed_units(&self) -> Result<Vec<ManagedUnit>, UnitError>;
}

/// Production `TransientUnitManager` shelling out to `systemd-run`/`systemctl`
/// as non-blocking subprocesses.
#[derive(Debug, Clone)]
pub struct SystemdRunUnitManager {
    systemd_run_path: PathBuf,
    systemctl_path: PathBuf,
}

impl SystemdRunUnitManager {
    /// `systemctl` is derived from the directory holding `systemd-run` (both
    /// ship in the systemd package's `bin/`).
    pub fn new(systemd_run_path: impl Into<PathBuf>) -> Self {
        let systemd_run_path = systemd_run_path.into();
        let systemctl_path = systemd_run_path
            .parent()
            .map(|dir| dir.join("systemctl"))
            .unwrap_or_else(|| PathBuf::from("systemctl"));
        Self {
            systemd_run_path,
            systemctl_path,
        }
    }

    pub fn systemd_run_path(&self) -> &PathBuf {
        &self.systemd_run_path
    }

    pub fn systemctl_path(&self) -> &PathBuf {
        &self.systemctl_path
    }
}

#[async_trait]
impl TransientUnitManager for SystemdRunUnitManager {
    async fn start_transient_unit(
        &self,
        slot: u32,
        ceiling_sec: u64,
        paths: &RunnerUnitPaths,
    ) -> Result<(), UnitError> {
        let mut cmd = tokio::process::Command::new(&self.systemd_run_path);
        cmd.args(systemd_run_argv(slot, ceiling_sec, &paths.exec_runner_path))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());

        let status = cmd.status().await.map_err(|_| UnitError::SpawnFailed)?;
        if status.success() {
            Ok(())
        } else {
            Err(UnitError::NonZeroExit)
        }
    }

    async fn stop_unit(&self, slot: u32) -> Result<(), UnitError> {
        let mut cmd = tokio::process::Command::new(&self.systemctl_path);
        cmd.arg("stop")
            .arg(unit_name(slot))
            .arg(workload_unit_name(slot))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let stop_status = cmd.status().await.map_err(|_| UnitError::SpawnFailed)?;
        let mut kill = tokio::process::Command::new(&self.systemctl_path);
        kill.arg("--system")
            .arg("--no-ask-password")
            .arg("--quiet")
            .arg("--kill-whom=all")
            .arg("--signal=SIGKILL")
            .arg("kill")
            .arg(workload_unit_name(slot))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let kill_status = kill.status().await.map_err(|_| UnitError::SpawnFailed)?;
        if stop_status.success() && kill_status.success() {
            Ok(())
        } else {
            Err(UnitError::NonZeroExit)
        }
    }

    async fn reset_failed(&self, slot: u32) -> Result<(), UnitError> {
        let mut cmd = tokio::process::Command::new(&self.systemctl_path);
        cmd.arg("reset-failed")
            .arg(unit_name(slot))
            .arg(workload_unit_name(slot))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let _ = cmd.status().await.map_err(|_| UnitError::SpawnFailed)?;
        Ok(())
    }

    async fn list_managed_units(&self) -> Result<Vec<ManagedUnit>, UnitError> {
        let mut cmd = tokio::process::Command::new(&self.systemctl_path);
        cmd.arg("list-units")
            .arg("--type=service")
            .arg("--all")
            .arg("--no-legend")
            .arg("--plain")
            .arg("nixling-exec-*.service")
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let output = cmd.output().await.map_err(|_| UnitError::SpawnFailed)?;
        if !output.status.success() {
            return Err(UnitError::NonZeroExit);
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let mut units = parse_managed_units(&text);
        if units.is_empty() {
            return Ok(units);
        }
        // Enrich with Slice + ExecStart + unit dependencies for identity
        // verification. The `show`
        // step is best-effort for LIVENESS but load-bearing for IDENTITY: a
        // failure (spawn error, non-zero exit, or a unit with no returned
        // block) leaves that unit's identity `Unqueried`, so an ACTIVE unit
        // classifies as `Unknown` (retry) rather than `Foreign` (destructive).
        // Only a successfully-read identity that mismatches is `Foreign`.
        let mut show = tokio::process::Command::new(&self.systemctl_path);
        show.arg("show")
            .arg("--no-pager")
            .arg("--property=Id,Slice,ExecStart,BindsTo,PartOf,After")
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        for unit in &units {
            show.arg(unit.name());
        }
        if let Ok(show_out) = show.output().await {
            if show_out.status.success() {
                let show_text = String::from_utf8_lossy(&show_out.stdout);
                let blocks = parse_show_blocks(&show_text);
                for unit in &mut units {
                    let name = unit.name();
                    if let Some(block) = blocks.iter().find(|b| b.id == name) {
                        unit.identity = UnitIdentity::Queried {
                            slice: block.slice.clone(),
                            exec_start: block.exec_start.clone(),
                            binds_to: block.binds_to.clone(),
                            part_of: block.part_of.clone(),
                            after: block.after.clone(),
                        };
                    }
                }
            }
        }
        Ok(units)
    }
}

/// One `systemctl show` property block (identity-verification input).
#[derive(Default, PartialEq, Eq)]
struct ShowEntry {
    id: String,
    slice: Option<String>,
    exec_start: Option<String>,
    binds_to: Option<String>,
    part_of: Option<String>,
    after: Option<String>,
}

impl fmt::Debug for ShowEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShowEntry")
            .field("id", &self.id)
            .field("slice", &self.slice.as_deref())
            .field("has_exec_start", &self.exec_start.is_some())
            .field("has_binds_to", &self.binds_to.is_some())
            .field("has_part_of", &self.part_of.is_some())
            .field("has_after", &self.after.is_some())
            .finish()
    }
}

/// Parse `systemctl show --property=Id,Slice,ExecStart,BindsTo,PartOf,After
/// UNIT...` output. Blocks
/// for distinct units are separated by a blank line. `ExecStart` renders as
/// `{ path=... ; argv[]=<abs> --serve-exec --slot NN ; ... }`; the raw value is
/// kept verbatim and decomposed structurally upstream (see
/// [`parse_exec_start`]) for the identity check.
fn parse_show_blocks(text: &str) -> Vec<ShowEntry> {
    let mut out = Vec::new();
    for block in text.split("\n\n") {
        let mut entry = ShowEntry::default();
        for line in block.lines() {
            if let Some(v) = line.strip_prefix("Id=") {
                entry.id = v.trim().to_owned();
            } else if let Some(v) = line.strip_prefix("Slice=") {
                let v = v.trim();
                if !v.is_empty() {
                    entry.slice = Some(v.to_owned());
                }
            } else if let Some(v) = line.strip_prefix("ExecStart=") {
                let v = v.trim();
                if !v.is_empty() && entry.exec_start.is_none() {
                    entry.exec_start = Some(v.to_owned());
                }
            } else if let Some(v) = line.strip_prefix("BindsTo=") {
                let v = v.trim();
                if !v.is_empty() {
                    entry.binds_to = Some(v.to_owned());
                }
            } else if let Some(v) = line.strip_prefix("PartOf=") {
                let v = v.trim();
                if !v.is_empty() {
                    entry.part_of = Some(v.to_owned());
                }
            } else if let Some(v) = line.strip_prefix("After=") {
                let v = v.trim();
                if !v.is_empty() {
                    entry.after = Some(v.to_owned());
                }
            }
        }
        if !entry.id.is_empty() {
            out.push(entry);
        }
    }
    out
}

/// Parse `systemctl list-units --plain --no-legend` output into the bounded set
/// of managed `nixling-exec-<NN>.service` units. Columns are
/// `UNIT LOAD ACTIVE SUB DESCRIPTION`; a unit is `active` iff its ACTIVE column
/// is `active` or `activating`.
fn parse_managed_units(text: &str) -> Vec<ManagedUnit> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut cols = line.split_whitespace();
        let Some(unit) = cols.next() else { continue };
        // systemctl prefixes a status glyph (e.g. "●") on failed/loaded units
        // in some locales; tolerate a leading non-unit token.
        let unit = if unit.ends_with(".service") {
            unit
        } else if let Some(next) = cols.clone().next() {
            if next.ends_with(".service") {
                cols.next();
                next
            } else {
                continue;
            }
        } else {
            continue;
        };
        let Some((slot, kind)) = parse_slot_from_unit(unit) else {
            continue;
        };
        // ACTIVE column (3rd after the unit name): LOAD ACTIVE SUB ...
        let _load = cols.next();
        let active_state = cols.next().unwrap_or("");
        let active = matches!(active_state, "active" | "activating");
        out.push(ManagedUnit {
            slot,
            kind,
            active,
            identity: UnitIdentity::Unqueried,
        });
    }
    out
}

/// Extract the slot/kind from `nixling-exec-<NN>.service` or
/// `nixling-exec-<NN>-w.service`; `None` if it does not match the bounded
/// slot-keyed names.
fn parse_slot_from_unit(unit: &str) -> Option<(u32, ManagedUnitKind)> {
    let rest = unit.strip_prefix("nixling-exec-")?;
    if let Some(digits) = rest.strip_suffix("-w.service") {
        return digits
            .parse::<u32>()
            .ok()
            .map(|slot| (slot, ManagedUnitKind::Workload));
    }
    let digits = rest.strip_suffix(".service")?;
    digits
        .parse::<u32>()
        .ok()
        .map(|slot| (slot, ManagedUnitKind::Runner))
}

/// Stable unit name for a slot: `nixling-exec-<NN>.service` (zero-padded). The
/// opaque exec id never appears in the unit name (journald cardinality bound).
pub fn unit_name(slot: u32) -> String {
    format!("nixling-exec-{slot:02}.service")
}

/// Stable workload unit name for a slot: `nixling-exec-<NN>-w.service`.
pub fn workload_unit_name(slot: u32) -> String {
    format!("nixling-exec-{slot:02}-w.service")
}

/// Build the full `systemd-run` argv (excluding the `systemd-run` binary
/// itself) for a slot's runner unit. The argv carries only controlled
/// constants: a slot-keyed `--unit`, the dedicated slice, fail-closed
/// stdio, and the runner invocation. The opaque exec id never appears. The
/// optional indefinite-runtime ceiling emits `RuntimeMaxSec` ONLY when the
/// operator opted in (`ceiling_sec > 0`); a value of 0 emits no such flag.
fn systemd_run_argv(slot: u32, ceiling_sec: u64, exec_runner_path: &Path) -> Vec<OsString> {
    let unit = format!("nixling-exec-{slot:02}");
    let slot_arg = format!("{slot:02}");
    let timeout_stop = format!(
        "TimeoutStopSec={}",
        crate::detached_registry::TIMEOUT_STOP_SEC
    );

    let mut argv: Vec<OsString> = vec![
        OsString::from(format!("--unit={unit}")),
        OsString::from("--slice=nixling-exec.slice"),
        OsString::from("-p"),
        OsString::from("User=root"),
        OsString::from("-p"),
        OsString::from("StandardInput=null"),
        OsString::from("-p"),
        OsString::from("StandardOutput=null"),
        OsString::from("-p"),
        OsString::from("StandardError=null"),
        OsString::from("-p"),
        OsString::from("KillMode=mixed"),
        OsString::from("-p"),
        OsString::from(timeout_stop),
        OsString::from("-p"),
        OsString::from("Type=exec"),
    ];
    // Optional indefinite-runtime ceiling: only emit RuntimeMaxSec when the
    // operator opted in (> 0). It MUST be strictly larger than the runner's own
    // control-watcher ceiling (== ceiling_sec) plus the full TERM->grace->KILL->
    // reap->status window so systemd's unit-level RuntimeMaxSec SIGTERM (which
    // the no-handler runner would die on immediately) only fires after the
    // runner already published `cancelled`.
    if ceiling_sec > 0 {
        let runtime_max = ceiling_sec
            .saturating_add(crate::detached_registry::TIMEOUT_STOP_SEC)
            .saturating_add(crate::detached_registry::RUNTIME_MAX_MARGIN_SEC);
        argv.push(OsString::from("-p"));
        argv.push(OsString::from(format!("RuntimeMaxSec={runtime_max}")));
    }
    argv.push(exec_runner_path.as_os_str().to_owned());
    argv.push(OsString::from("--serve-exec"));
    argv.push(OsString::from("--slot"));
    argv.push(OsString::from(slot_arg));
    argv
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_name_is_slot_keyed_and_id_free() {
        assert_eq!(unit_name(0), "nixling-exec-00.service");
        assert_eq!(unit_name(31), "nixling-exec-31.service");
    }

    #[test]
    fn systemd_run_argv_is_slot_keyed_and_id_free() {
        let runner = PathBuf::from("/nix/store/abc-nixling-exec-runner/bin/nixling-exec-runner");
        let argv = systemd_run_argv(7, 0, &runner);
        let joined: Vec<String> = argv
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(joined.contains(&"--unit=nixling-exec-07".to_string()));
        assert!(joined.contains(&"--slice=nixling-exec.slice".to_string()));
        assert!(joined.contains(&"--serve-exec".to_string()));
        // Slot is the only per-exec discriminator; no opaque id leaks in.
        let slot_idx = joined.iter().position(|a| a == "--slot").unwrap();
        assert_eq!(joined[slot_idx + 1], "07");
        assert!(joined.iter().all(|a| !a.contains("exec_id")));
    }

    #[test]
    fn systemd_run_argv_emits_runtime_max_sec_only_when_set() {
        let runner = PathBuf::from("/nix/store/abc/bin/nixling-exec-runner");
        // ceiling 0 => indefinite => no RuntimeMaxSec flag.
        let none = systemd_run_argv(3, 0, &runner);
        assert!(none
            .iter()
            .all(|a| !a.to_string_lossy().starts_with("RuntimeMaxSec=")));
        // ceiling > 0 => the backstop is emitted, and STRICTLY larger than the
        // runner's own ceiling so systemd does not SIGKILL before the runner
        // writes `cancelled`.
        let ceiling = 3600;
        let some = systemd_run_argv(3, ceiling, &runner);
        let runtime_max: u64 = some
            .iter()
            .find_map(|a| {
                a.to_string_lossy()
                    .strip_prefix("RuntimeMaxSec=")
                    .and_then(|v| v.parse().ok())
            })
            .expect("RuntimeMaxSec emitted when ceiling > 0");
        assert!(
            runtime_max > ceiling,
            "RuntimeMaxSec ({runtime_max}) must exceed the runner ceiling ({ceiling})"
        );
        assert_eq!(
            runtime_max,
            ceiling
                + crate::detached_registry::TIMEOUT_STOP_SEC
                + crate::detached_registry::RUNTIME_MAX_MARGIN_SEC
        );
    }

    #[test]
    fn systemctl_is_derived_next_to_systemd_run() {
        let mgr = SystemdRunUnitManager::new("/run/current-system/sw/bin/systemd-run");
        assert_eq!(
            mgr.systemctl_path(),
            &PathBuf::from("/run/current-system/sw/bin/systemctl")
        );
    }

    #[test]
    fn parse_slot_from_unit_matches_only_bounded_names() {
        assert_eq!(
            parse_slot_from_unit("nixling-exec-00.service"),
            Some((0, ManagedUnitKind::Runner))
        );
        assert_eq!(
            parse_slot_from_unit("nixling-exec-31.service"),
            Some((31, ManagedUnitKind::Runner))
        );
        assert_eq!(
            parse_slot_from_unit("nixling-exec-07.service"),
            Some((7, ManagedUnitKind::Runner))
        );
        assert_eq!(
            parse_slot_from_unit("nixling-exec-07-w.service"),
            Some((7, ManagedUnitKind::Workload))
        );
        assert_eq!(parse_slot_from_unit("other.service"), None);
        assert_eq!(parse_slot_from_unit("nixling-exec-.service"), None);
        assert_eq!(parse_slot_from_unit("nixling-exec-xx.service"), None);
    }

    #[test]
    fn parse_managed_units_reads_active_column() {
        let text = "\
nixling-exec-00.service loaded active   running Detached exec 00
nixling-exec-00-w.service loaded active running Detached exec 00 workload
nixling-exec-01.service loaded inactive dead    Detached exec 01
nixling-exec-02.service loaded activating start Detached exec 02
other.service           loaded active   running Unrelated
";
        let mut units = parse_managed_units(text);
        units.sort_by_key(|u| (u.slot, u.kind == ManagedUnitKind::Workload));
        assert_eq!(
            units,
            vec![
                ManagedUnit {
                    slot: 0,
                    kind: ManagedUnitKind::Runner,
                    active: true,
                    identity: UnitIdentity::Unqueried
                },
                ManagedUnit {
                    slot: 0,
                    kind: ManagedUnitKind::Workload,
                    active: true,
                    identity: UnitIdentity::Unqueried
                },
                ManagedUnit {
                    slot: 1,
                    kind: ManagedUnitKind::Runner,
                    active: false,
                    identity: UnitIdentity::Unqueried
                },
                ManagedUnit {
                    slot: 2,
                    kind: ManagedUnitKind::Runner,
                    active: true,
                    identity: UnitIdentity::Unqueried
                },
            ]
        );
    }

    #[test]
    fn parse_managed_units_tolerates_leading_status_glyph() {
        let text = "● nixling-exec-05.service loaded failed failed Detached exec 05\n";
        let units = parse_managed_units(text);
        assert_eq!(
            units,
            vec![ManagedUnit {
                slot: 5,
                kind: ManagedUnitKind::Runner,
                active: false,
                identity: UnitIdentity::Unqueried
            }]
        );
    }

    #[test]
    fn parse_show_blocks_extracts_id_slice_and_exec_start() {
        let text = "\
Id=nixling-exec-07.service
Slice=nixling-exec.slice
ExecStart={ path=/nix/store/abc/bin/nixling-exec-runner ; argv[]=/nix/store/abc/bin/nixling-exec-runner --serve-exec --slot 07 ; ignore_errors=no }
BindsTo=nixling-exec-07.service
PartOf=nixling-exec-07.service
After=nixling-exec-07.service

Id=nixling-exec-08.service
Slice=other.slice
ExecStart={ path=/bin/false ; argv[]=/bin/false ; ignore_errors=no }
";
        let blocks = parse_show_blocks(text);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].id, "nixling-exec-07.service");
        assert_eq!(blocks[0].slice.as_deref(), Some("nixling-exec.slice"));
        assert!(blocks[0]
            .exec_start
            .as_deref()
            .unwrap()
            .contains("--slot 07"));
        assert_eq!(
            blocks[0].binds_to.as_deref(),
            Some("nixling-exec-07.service")
        );
        assert_eq!(
            blocks[0].part_of.as_deref(),
            Some("nixling-exec-07.service")
        );
        assert_eq!(blocks[0].after.as_deref(), Some("nixling-exec-07.service"));
        assert_eq!(blocks[1].slice.as_deref(), Some("other.slice"));
    }

    #[test]
    fn parse_show_blocks_keeps_only_first_exec_start_and_skips_empty() {
        let text = "\
Id=nixling-exec-00.service
Slice=
ExecStart=
";
        let blocks = parse_show_blocks(text);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].id, "nixling-exec-00.service");
        assert_eq!(blocks[0].slice, None);
        assert_eq!(blocks[0].exec_start, None);
    }

    #[test]
    fn parse_exec_start_extracts_exe_and_argv_tokens() {
        let parsed = parse_exec_start(
            "{ path=/nix/store/abc/bin/nixling-exec-runner ; \
             argv[]=/nix/store/abc/bin/nixling-exec-runner --serve-exec --slot 07 ; \
             ignore_errors=no }",
        )
        .expect("structured ExecStart parses");
        assert_eq!(parsed.exe, "/nix/store/abc/bin/nixling-exec-runner");
        assert_eq!(
            parsed.argv,
            vec![
                "/nix/store/abc/bin/nixling-exec-runner".to_owned(),
                "--serve-exec".to_owned(),
                "--slot".to_owned(),
                "07".to_owned(),
            ]
        );
    }

    #[test]
    fn parse_exec_start_preserves_quoted_argv_token_boundaries() {
        let parsed = parse_exec_start(
            r#"{ path=/run/current-system/sw/bin/bash ; argv[]=/run/current-system/sw/bin/bash -l -c "exec \"$@\"" nl-exec "/bin/echo" "hello world" ; ignore_errors=no }"#,
        )
        .expect("quoted argv parses");
        assert_eq!(
            parsed.argv,
            vec![
                "/run/current-system/sw/bin/bash".to_owned(),
                "-l".to_owned(),
                "-c".to_owned(),
                r#"exec "$@""#.to_owned(),
                "nl-exec".to_owned(),
                "/bin/echo".to_owned(),
                "hello world".to_owned(),
            ]
        );
    }

    #[test]
    fn parse_exec_start_rejects_unstructured_or_empty() {
        // No `path=` field at all.
        assert_eq!(
            parse_exec_start("/usr/bin/evil --serve-exec --slot 07"),
            None
        );
        // Empty executable path.
        assert_eq!(
            parse_exec_start("{ path= ; argv[]=x ; ignore_errors=no }"),
            None
        );
    }

    #[test]
    fn debug_impls_redact_exec_start_argv() {
        let secret = "--token=super-secret";
        let identity = UnitIdentity::Queried {
            slice: Some("nixling-exec.slice".to_owned()),
            exec_start: Some(format!(
                "{{ path=/bin/bash ; argv[]=/bin/bash -lc {secret} ; ignore_errors=no }}"
            )),
            binds_to: Some("nixling-exec-03.service".to_owned()),
            part_of: Some("nixling-exec-03.service".to_owned()),
            after: Some("nixling-exec-03.service".to_owned()),
        };
        let unit = ManagedUnit {
            slot: 3,
            kind: ManagedUnitKind::Workload,
            active: true,
            identity: identity.clone(),
        };
        let show = ShowEntry {
            id: "nixling-exec-03-w.service".to_owned(),
            slice: Some("nixling-exec.slice".to_owned()),
            exec_start: Some(format!(
                "{{ path=/bin/bash ; argv[]=/bin/bash -lc {secret} ; ignore_errors=no }}"
            )),
            binds_to: Some("nixling-exec-03.service".to_owned()),
            part_of: Some("nixling-exec-03.service".to_owned()),
            after: Some("nixling-exec-03.service".to_owned()),
        };
        for rendered in [
            format!("{identity:?}"),
            format!("{unit:?}"),
            format!("{show:?}"),
        ] {
            assert!(
                !rendered.contains(secret),
                "Debug output leaked ExecStart argv: {rendered}"
            );
            assert!(
                rendered.contains("has_exec_start: true"),
                "Debug output should expose only metadata: {rendered}"
            );
        }
    }
}
