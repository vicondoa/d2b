//! `d2b host validate` composite
//! preflight verb.
//!
//! This module ships the operator-facing one-command preflight that
//! must run after a fresh `nixos-rebuild switch` to record the
//! per-wave validation evidence the readiness assertions consume.
//! (`d2b.daemonExperimental.enable` defaults `true` and is no
//! longer evidence-auto-flipped — there is no default to flip — but it
//! still functionally gates the daemon control plane; setting it
//! `false` reverts the host to the unsupported pre-daemon legacy
//! state.) It iterates the known
//! readiness waves (mirrored from
//! `nixos-modules/options-daemon.nix:readinessWaveSpecs`) in a
//! deterministic order; for each wave it discovers the per-wave
//! Layer-2 validator scripts shipped in `tests/` and reports their
//! presence + executability.
//!
//! Modes:
//!   * `--dry-run` (default-able): inventory only. Reports what WOULD
//!     be validated, no host mutation, no evidence write.
//!   * `--apply`: writes the canonical evidence record
//!     `/var/lib/d2b/validated/<wave>.json` for every wave whose
//!     declared validators are all present on disk. The record shape
//!     is the readiness contract enforced by
//!     `options-daemon.nix:validationEvidencePresent`:
//!     `{"wave": "<wave>", "timestamp": "<UTC ISO-8601>", "operatorSignature": "<sha256:...>"}`
//!     The operator signature is computed from
//!     `hostname | wave | bundle_path | timestamp` unless the
//!     operator overrides it via `--operator-signature <sig>`.
//!
//! The per-wave `validated = true` readiness assertions consume these
//! evidence files (the historical daemonExperimental auto-flip gate is
//! retired) — see the Critical-subsystems "Control plane" row in
//! AGENTS.md and `docs/reference/default-switch-and-deprecation.md`.
//!
//! This verb is intentionally a thin orchestrator: it does NOT
//! execute the per-wave shell validators itself (those are Layer-2
//! integration tests that frequently require live host state, sudo,
//! and external hardware). Instead, it lets the operator attest that
//! the validators were run by issuing the evidence record as a
//! single composite operation. Per-wave validators that already write
//! their own evidence records (e.g. `tests/minijail-validator-swtpm.sh`
//! → `p1-swtpm.json`) continue to do so; this verb is the umbrella
//! preflight that produces the per-wave `<wave>.json` records the
//! readiness option consumes.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Canonical location the default-switch auto-flip gate reads from.
/// Mirrors `nixos-modules/options-daemon.nix:validationEvidenceDir`.
pub const DEFAULT_EVIDENCE_DIR: &str = "/var/lib/d2b/validated";

/// One known readiness wave plus the per-wave Layer-2 validator scripts
/// the operator is expected to have exercised before
/// attesting via `host validate --apply`.
///
/// The wave-name vocabulary MUST stay byte-identical with
/// `readinessWaveSpecs` in `nixos-modules/options-daemon.nix` —
/// `tests/host-validate-verb-eval.sh` enforces parity.
#[derive(Debug, Clone, Copy)]
pub struct WaveSpec {
    /// Wave id, e.g. `"p1"` or `"w5Fu"`. Matches the file basename the
    /// readiness option consumes (`/var/lib/d2b/validated/<wave>.json`).
    pub wave: &'static str,
    /// Short human-readable summary of what the wave covers.
    pub summary: &'static str,
    /// Per-wave Layer-2 validator script basenames, relative to
    /// `tests/`. Discovery checks file existence + readability.
    pub validators: &'static [&'static str],
}

/// Canonical, deterministic wave order. Sequencing matches the
/// natural rollout (`w*Fu` follow-ups → `p0`..`p7` phase work) so
/// human readers can scan the report top-to-bottom.
pub const WAVE_CATALOG: &[WaveSpec] = &[
    WaveSpec {
        wave: "w4Fu",
        summary: "Headless daemon + supervisor path (Ubuntu Tier-1 smoke).",
        validators: &["d2bd-startup-smoke.sh"],
    },
    WaveSpec {
        wave: "w5Fu",
        summary: "Minijail profiles + GPU/audio/video argv generators (hardware smoke).",
        validators: &["hardware-smoke-gpu-yubikey.sh"],
    },
    WaveSpec {
        wave: "w6Fu",
        summary: "USBIP live executors + per-busid lock (hardware smoke).",
        validators: &[
            "hardware-smoke-gpu-yubikey.sh",
            "usbip-state-machine-eval.sh",
        ],
    },
    WaveSpec {
        wave: "w7Fu",
        summary: "Store-lifecycle verbs + admin auth.",
        validators: &["per-vm-state-ownership-eval.sh"],
    },
    WaveSpec {
        wave: "w8Fu",
        summary: "Keys/trust/rotate-known-host live wiring.",
        validators: &["ssh-host-key-preflight-eval.sh"],
    },
    WaveSpec {
        wave: "w9Fu",
        summary: "Host install + migrate live broker ops.",
        validators: &["harness-ubuntu-eval.sh"],
    },
    WaveSpec {
        wave: "p0",
        summary: "Daemon-only foundation (broker socket-activation + bundle digest verify).",
        validators: &[
            "broker-socket-activation-eval.sh",
            "broker-caps-eval.sh",
            "d2bd-startup-smoke.sh",
        ],
    },
    WaveSpec {
        wave: "p0Fu",
        summary: "Cgroup delegation + per-artifact hash verification.",
        validators: &["broker-bundle-path-eval.sh"],
    },
    WaveSpec {
        wave: "p1",
        summary: "Per-role minijail profiles + byte-parity argv generators.",
        validators: &[
            "minijail-validator-cloud-hypervisor.sh",
            "minijail-validator-virtiofsd.sh",
            "minijail-validator-swtpm.sh",
            "minijail-validator-gpu.sh",
            "minijail-validator-audio.sh",
            "minijail-validator-video.sh",
            "minijail-validator-vsock-relay.sh",
            "minijail-validator-usbip.sh",
            "minijail-validator-otel-host-bridge.sh",
        ],
    },
    WaveSpec {
        wave: "p2",
        summary: "Daemon-side host-prep + ownership matrix + manifestVersion=4.",
        validators: &[
            "per-vm-state-ownership-eval.sh",
            "daemon-autostart-eval.sh",
            "host-prep-dag-eval.sh",
        ],
    },
    WaveSpec {
        wave: "p3",
        summary: "Host singletons retired + daemon health endpoint.",
        validators: &[
            "observability-eval.sh",
            "daemon-metrics-eval.sh",
            "usbip-state-machine-eval.sh",
        ],
    },
    WaveSpec {
        wave: "p4",
        summary: "VM start/stop/restart/list daemon-native end-to-end.",
        validators: &["cli-vm-verbs-eval.sh", "desktop-wrapper-contract-eval.sh"],
    },
    WaveSpec {
        wave: "p5",
        summary: "First-run validation UX (this verb + daemon auto-write on first op).",
        validators: &["host-validate-verb-eval.sh"],
    },
    WaveSpec {
        wave: "p6",
        summary: "Legacy systemd template emission + bash CLI removed.",
        validators: &[],
    },
    WaveSpec {
        wave: "p7",
        summary: "Docs blast-radius + v1.0 cut.",
        validators: &[],
    },
    WaveSpec {
        wave: "p0Cb",
        summary: "Clipboard authority (d2b-clipd + picker protocol handshake smoke).",
        validators: &["clipboard-picker-smoke.sh"],
    },
];

/// Per-wave status reported by both dry-run and apply modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WaveStatus {
    /// Every declared validator script is present on disk.
    Ready,
    /// At least one declared validator script is missing.
    Missing,
    /// No validators are declared for this wave (informational —
    /// e.g. `p6`/`p7` whose readiness signal is gate-output, not a
    /// per-host script).
    NoValidators,
    /// Apply mode only: evidence record was written successfully.
    Attested,
    /// Apply mode only: evidence write was skipped because the wave
    /// is `Missing` or because `--wave <other>` filtered it out.
    Skipped,
    /// Apply mode only: evidence write failed (e.g. permission
    /// denied). The detail field carries the underlying error.
    WriteFailed,
}

impl WaveStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            WaveStatus::Ready => "ready",
            WaveStatus::Missing => "missing",
            WaveStatus::NoValidators => "no-validators",
            WaveStatus::Attested => "attested",
            WaveStatus::Skipped => "skipped",
            WaveStatus::WriteFailed => "write-failed",
        }
    }
}

#[derive(Debug, Clone)]
pub struct WaveReport {
    pub wave: &'static str,
    pub summary: &'static str,
    pub status: WaveStatus,
    /// Per-validator presence map: `(basename, present)`.
    pub validators: Vec<(String, bool)>,
    /// Human-readable detail (e.g. evidence path written, error
    /// reason).
    pub detail: String,
    /// On `Attested`, the absolute evidence path. Otherwise `None`.
    pub evidence_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct ValidateReport {
    pub mode: ValidateMode,
    pub evidence_dir: PathBuf,
    pub scripts_dir: PathBuf,
    pub waves: Vec<WaveReport>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidateMode {
    DryRun,
    Apply,
}

impl ValidateMode {
    fn as_str(self) -> &'static str {
        match self {
            ValidateMode::DryRun => "dry-run",
            ValidateMode::Apply => "apply",
        }
    }
}

/// Inputs to a `host validate` invocation. All paths are absolute or
/// resolved by `run_host_validate` against the process cwd.
#[derive(Debug, Clone)]
pub struct ValidateRequest {
    pub mode: ValidateMode,
    /// Where per-wave evidence records are written
    /// (`<wave>.json`). Defaults to `DEFAULT_EVIDENCE_DIR`.
    pub evidence_dir: PathBuf,
    /// Where the per-wave validator scripts live. Defaults to a
    /// best-effort search; tests override via the
    /// `D2B_VALIDATE_SCRIPTS_DIR` env knob.
    pub scripts_dir: PathBuf,
    /// Optional single-wave filter; when set, only that wave is
    /// reported (and, in apply mode, only that wave's evidence is
    /// written).
    pub only_wave: Option<String>,
    /// Optional operator-supplied signature; otherwise the verb
    /// computes a deterministic per-wave signature.
    pub operator_signature: Option<String>,
    /// Override the timestamp (used by tests). When `None`, the
    /// current UTC time is used.
    pub timestamp_override: Option<String>,
    /// Override the hostname used in the default operator signature
    /// (used by tests). When `None`, `gethostname()` is consulted.
    pub hostname_override: Option<String>,
}

impl ValidateRequest {
    pub fn from_env_defaults(mode: ValidateMode) -> Self {
        let evidence_dir = std::env::var_os("D2B_VALIDATE_EVIDENCE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_EVIDENCE_DIR));
        let scripts_dir = resolve_default_scripts_dir();
        Self {
            mode,
            evidence_dir,
            scripts_dir,
            only_wave: None,
            operator_signature: None,
            timestamp_override: None,
            hostname_override: None,
        }
    }
}

fn resolve_default_scripts_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("D2B_VALIDATE_SCRIPTS_DIR") {
        return PathBuf::from(p);
    }
    // Production: NixOS module installs validator scripts under the
    // system profile share dir.
    let installed = PathBuf::from("/run/current-system/sw/share/d2b/tests");
    if installed.is_dir() {
        return installed;
    }
    // Dev fallback: assume cwd is repo root or a child directory
    // therein.
    PathBuf::from("tests")
}

/// Top-level entry point invoked from `cmd_host_validate` in
/// `lib.rs`.
pub fn run_host_validate(req: &ValidateRequest) -> ValidateReport {
    let mut waves = Vec::with_capacity(WAVE_CATALOG.len());
    for spec in WAVE_CATALOG {
        if let Some(only) = &req.only_wave
            && only != spec.wave
        {
            waves.push(skip_wave(spec, "filtered by --wave"));
            continue;
        }
        let validators = inventory_validators(spec, &req.scripts_dir);
        let all_present = !validators.is_empty() && validators.iter().all(|(_, p)| *p);
        let status = if spec.validators.is_empty() {
            WaveStatus::NoValidators
        } else if all_present {
            WaveStatus::Ready
        } else {
            WaveStatus::Missing
        };
        let detail = render_inventory_detail(&validators, status, &req.scripts_dir);
        let report = WaveReport {
            wave: spec.wave,
            summary: spec.summary,
            status,
            validators,
            detail,
            evidence_path: None,
        };
        let report = if matches!(req.mode, ValidateMode::Apply) {
            maybe_write_evidence(report, spec, req)
        } else {
            report
        };
        waves.push(report);
    }
    ValidateReport {
        mode: req.mode,
        evidence_dir: req.evidence_dir.clone(),
        scripts_dir: req.scripts_dir.clone(),
        waves,
    }
}

fn skip_wave(spec: &WaveSpec, reason: &str) -> WaveReport {
    WaveReport {
        wave: spec.wave,
        summary: spec.summary,
        status: WaveStatus::Skipped,
        validators: Vec::new(),
        detail: reason.to_owned(),
        evidence_path: None,
    }
}

fn inventory_validators(spec: &WaveSpec, scripts_dir: &Path) -> Vec<(String, bool)> {
    spec.validators
        .iter()
        .map(|name| {
            let path = scripts_dir.join(name);
            (name.to_string(), path.is_file())
        })
        .collect()
}

fn render_inventory_detail(
    validators: &[(String, bool)],
    status: WaveStatus,
    scripts_dir: &Path,
) -> String {
    match status {
        WaveStatus::Ready => {
            format!(
                "{} validator(s) present in {}",
                validators.len(),
                scripts_dir.display()
            )
        }
        WaveStatus::Missing => {
            let missing: Vec<&str> = validators
                .iter()
                .filter_map(|(n, p)| if !*p { Some(n.as_str()) } else { None })
                .collect();
            format!(
                "missing {}/{} validator(s) in {}: {}",
                missing.len(),
                validators.len(),
                scripts_dir.display(),
                missing.join(", ")
            )
        }
        WaveStatus::NoValidators => {
            "wave has no per-host Layer-2 validator scripts (gate-output only)".to_owned()
        }
        _ => String::new(),
    }
}

fn maybe_write_evidence(
    mut report: WaveReport,
    spec: &WaveSpec,
    req: &ValidateRequest,
) -> WaveReport {
    match report.status {
        WaveStatus::Ready => {
            let payload = build_evidence_payload(spec, req);
            let path = req.evidence_dir.join(format!("{}.json", spec.wave));
            match write_evidence(&path, &payload) {
                Ok(()) => {
                    report.status = WaveStatus::Attested;
                    report.detail = format!("evidence written to {}", path.display());
                    report.evidence_path = Some(path);
                }
                Err(err) => {
                    report.status = WaveStatus::WriteFailed;
                    report.detail =
                        format!("failed to write evidence to {}: {}", path.display(), err);
                }
            }
        }
        WaveStatus::Missing => {
            report.detail = format!(
                "evidence NOT written: {} (run the listed validators before re-attesting)",
                report.detail
            );
            // Status stays `Missing` — operator must fix.
        }
        WaveStatus::NoValidators => {
            // No host-local validator → no operator-attested evidence is
            // meaningful for this wave. Leave the record absent so the
            // default-switch gate falls back to gate-output-only logic.
            report.detail = format!(
                "{} — operator attestation not applicable; rely on Layer-1 gate output",
                report.detail
            );
        }
        _ => {}
    }
    report
}

fn build_evidence_payload(spec: &WaveSpec, req: &ValidateRequest) -> Value {
    let timestamp = req
        .timestamp_override
        .clone()
        .unwrap_or_else(current_utc_timestamp);
    let signature = req
        .operator_signature
        .clone()
        .unwrap_or_else(|| compute_operator_signature(spec.wave, req, &timestamp));
    json!({
        "wave": spec.wave,
        "timestamp": timestamp,
        "operatorSignature": signature,
    })
}

fn write_evidence(path: &Path, payload: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("mkdir -p {}: {err}", parent.display()))?;
    }
    let mut rendered =
        serde_json::to_string_pretty(payload).map_err(|err| format!("serialize: {err}"))?;
    rendered.push('\n');
    std::fs::write(path, rendered).map_err(|err| format!("write: {err}"))?;
    Ok(())
}

fn current_utc_timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // ISO-8601 UTC formatter without bringing in chrono.
    format_iso8601_utc(now as i64)
}

fn format_iso8601_utc(epoch_secs: i64) -> String {
    // Civil-from-days algorithm (Howard Hinnant).
    let days = epoch_secs.div_euclid(86_400);
    let secs_of_day = epoch_secs.rem_euclid(86_400);
    let hour = secs_of_day / 3600;
    let minute = (secs_of_day % 3600) / 60;
    let second = secs_of_day % 60;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y, m, d, hour, minute, second
    )
}

fn compute_operator_signature(wave: &str, req: &ValidateRequest, timestamp: &str) -> String {
    let hostname = req.hostname_override.clone().unwrap_or_else(read_hostname);
    let scripts = req.scripts_dir.display().to_string();
    let input = format!("{hostname}|{wave}|{scripts}|{timestamp}");
    format!("sha256:{}", sha256_hex(input.as_bytes()))
}

fn read_hostname() -> String {
    // Avoid an extra crate dep — read /etc/hostname or fall back to
    // the HOSTNAME env var.
    if let Ok(text) = std::fs::read_to_string("/etc/hostname") {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            return trimmed.to_owned();
        }
    }
    std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown-host".to_owned())
}

// ---------------------------------------------------------------
// Minimal in-tree SHA-256 (no extra crate dep).
// FIPS-180-4 reference. Used ONLY for the operator-signature label —
// not a cryptographic boundary; the load-bearing default-switch trust
// signal is "evidence file exists with the canonical schema" (see
// options-daemon.nix:validationEvidencePresent).
// ---------------------------------------------------------------

fn sha256_hex(data: &[u8]) -> String {
    let digest = sha256(data);
    let mut out = String::with_capacity(64);
    for b in digest {
        let _ = std::fmt::Write::write_fmt(&mut out, format_args!("{:02x}", b));
    }
    out
}

#[allow(clippy::many_single_char_names)]
fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let bitlen = (data.len() as u64).wrapping_mul(8);
    let mut buf = Vec::with_capacity(data.len() + 72);
    buf.extend_from_slice(data);
    buf.push(0x80);
    while buf.len() % 64 != 56 {
        buf.push(0);
    }
    buf.extend_from_slice(&bitlen.to_be_bytes());
    for chunk in buf.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut a = h[0];
        let mut b = h[1];
        let mut c = h[2];
        let mut d = h[3];
        let mut e = h[4];
        let mut f = h[5];
        let mut g = h[6];
        let mut hh = h[7];
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

// ---------------------------------------------------------------
// Renderers
// ---------------------------------------------------------------

pub fn render_summary(report: &ValidateReport) -> Value {
    let waves: Vec<Value> = report
        .waves
        .iter()
        .map(|w| {
            json!({
                "wave": w.wave,
                "summary": w.summary,
                "status": w.status.as_str(),
                "validators": w
                    .validators
                    .iter()
                    .map(|(n, p)| json!({ "name": n, "present": *p }))
                    .collect::<Vec<_>>(),
                "detail": w.detail,
                "evidencePath": w
                    .evidence_path
                    .as_ref()
                    .map(|p| p.display().to_string()),
            })
        })
        .collect();
    let counts = tally(&report.waves);
    json!({
        "command": "host validate",
        "mode": report.mode.as_str(),
        "evidenceDir": report.evidence_dir.display().to_string(),
        "scriptsDir": report.scripts_dir.display().to_string(),
        "waves": waves,
        "summary": counts,
        "exitCode": exit_code(report),
    })
}

pub fn render_human(report: &ValidateReport) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let counts = tally(&report.waves);
    let _ = writeln!(
        out,
        "host validate --{}: scripts_dir={} evidence_dir={}",
        report.mode.as_str(),
        report.scripts_dir.display(),
        report.evidence_dir.display(),
    );
    let _ = writeln!(
        out,
        "  summary: ready={} attested={} missing={} no-validators={} skipped={} write-failed={}",
        counts["ready"],
        counts["attested"],
        counts["missing"],
        counts["noValidators"],
        counts["skipped"],
        counts["writeFailed"],
    );
    for w in &report.waves {
        let _ = writeln!(out, "  [{}] {} — {}", w.status.as_str(), w.wave, w.detail);
    }
    out
}

fn tally(waves: &[WaveReport]) -> serde_json::Map<String, Value> {
    let mut m = serde_json::Map::new();
    let mut ready = 0u64;
    let mut attested = 0u64;
    let mut missing = 0u64;
    let mut no_validators = 0u64;
    let mut skipped = 0u64;
    let mut write_failed = 0u64;
    for w in waves {
        match w.status {
            WaveStatus::Ready => ready += 1,
            WaveStatus::Attested => attested += 1,
            WaveStatus::Missing => missing += 1,
            WaveStatus::NoValidators => no_validators += 1,
            WaveStatus::Skipped => skipped += 1,
            WaveStatus::WriteFailed => write_failed += 1,
        }
    }
    m.insert("ready".into(), Value::from(ready));
    m.insert("attested".into(), Value::from(attested));
    m.insert("missing".into(), Value::from(missing));
    m.insert("noValidators".into(), Value::from(no_validators));
    m.insert("skipped".into(), Value::from(skipped));
    m.insert("writeFailed".into(), Value::from(write_failed));
    m
}

pub fn exit_code(report: &ValidateReport) -> i32 {
    // Apply mode: any write-failure is exit 1.
    // Any wave still `Missing` after apply is exit 78 (operator must
    // re-run after running the per-wave validator).
    // Dry-run mode: `Missing` is not an error — it's diagnostic.
    let mut has_write_failed = false;
    let mut has_missing = false;
    for w in &report.waves {
        match w.status {
            WaveStatus::WriteFailed => has_write_failed = true,
            WaveStatus::Missing => has_missing = true,
            _ => {}
        }
    }
    if has_write_failed {
        return 1;
    }
    if matches!(report.mode, ValidateMode::Apply) && has_missing {
        return 78;
    }
    0
}

// ---------------------------------------------------------------
// Tests
// ---------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_vector_abc() {
        // FIPS 180-4 §B.1 known vector.
        let got = sha256_hex(b"abc");
        assert_eq!(
            got,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn sha256_known_vector_empty() {
        let got = sha256_hex(b"");
        assert_eq!(
            got,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn iso8601_epoch_zero() {
        assert_eq!(format_iso8601_utc(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn iso8601_known_2025() {
        // 2025-11-15T10:50:00Z = 1763203800.
        assert_eq!(format_iso8601_utc(1_763_203_800), "2025-11-15T10:50:00Z");
    }

    #[test]
    fn wave_catalog_ids_match_options_daemon_nix() {
        // The readiness gate vocabulary lives in
        // nixos-modules/options-daemon.nix:readinessWaveSpecs.
        // Keep them in sync: the eval gate
        // tests/host-validate-verb-eval.sh enforces this from the
        // other direction.
        let expected = [
            "w4Fu", "w5Fu", "w6Fu", "w7Fu", "w8Fu", "w9Fu", "p0", "p0Fu", "p1", "p2", "p3", "p4",
            "p5", "p6", "p7", "p0Cb",
        ];
        let got: Vec<&str> = WAVE_CATALOG.iter().map(|w| w.wave).collect();
        assert_eq!(got, expected);
    }

    #[test]
    fn dry_run_reports_missing_when_scripts_absent() {
        let tmp = tempdir();
        let req = ValidateRequest {
            mode: ValidateMode::DryRun,
            evidence_dir: tmp.join("evidence"),
            scripts_dir: tmp.join("scripts-empty"),
            only_wave: None,
            operator_signature: None,
            timestamp_override: Some("2025-11-15T10:30:00Z".to_owned()),
            hostname_override: Some("test-host".to_owned()),
        };
        let report = run_host_validate(&req);
        // Every wave with declared validators should be Missing;
        // p6/p7 should be NoValidators.
        let p1 = report.waves.iter().find(|w| w.wave == "p1").unwrap();
        assert_eq!(p1.status, WaveStatus::Missing);
        let p6 = report.waves.iter().find(|w| w.wave == "p6").unwrap();
        assert_eq!(p6.status, WaveStatus::NoValidators);
        // Dry-run on Missing is exit 0.
        assert_eq!(exit_code(&report), 0);
        // No evidence written.
        assert!(!tmp.join("evidence").exists());
    }

    #[test]
    fn apply_writes_canonical_evidence_for_ready_waves() {
        let tmp = tempdir();
        let scripts = tmp.join("scripts");
        let evidence = tmp.join("evidence");
        std::fs::create_dir_all(&scripts).unwrap();
        // Stage the validator for p5 only.
        std::fs::write(scripts.join("host-validate-verb-eval.sh"), "stub\n").unwrap();
        let req = ValidateRequest {
            mode: ValidateMode::Apply,
            evidence_dir: evidence.clone(),
            scripts_dir: scripts,
            only_wave: Some("p5".to_owned()),
            operator_signature: None,
            timestamp_override: Some("2025-11-15T10:30:00Z".to_owned()),
            hostname_override: Some("test-host".to_owned()),
        };
        let report = run_host_validate(&req);
        let p5 = report.waves.iter().find(|w| w.wave == "p5").unwrap();
        assert_eq!(p5.status, WaveStatus::Attested);
        let written = std::fs::read_to_string(evidence.join("p5.json")).unwrap();
        let v: Value = serde_json::from_str(&written).unwrap();
        assert_eq!(v["wave"], "p5");
        assert_eq!(v["timestamp"], "2025-11-15T10:30:00Z");
        assert!(
            v["operatorSignature"]
                .as_str()
                .unwrap()
                .starts_with("sha256:"),
            "signature shape: {}",
            v["operatorSignature"]
        );
        // p4 etc. should be Skipped because of the --wave filter.
        let p4 = report.waves.iter().find(|w| w.wave == "p4").unwrap();
        assert_eq!(p4.status, WaveStatus::Skipped);
        assert_eq!(exit_code(&report), 0);
    }

    #[test]
    fn apply_with_missing_validators_refuses_and_exits_78() {
        let tmp = tempdir();
        let req = ValidateRequest {
            mode: ValidateMode::Apply,
            evidence_dir: tmp.join("evidence"),
            scripts_dir: tmp.join("scripts-empty"),
            only_wave: Some("p1".to_owned()),
            operator_signature: None,
            timestamp_override: Some("2025-11-15T10:30:00Z".to_owned()),
            hostname_override: Some("test-host".to_owned()),
        };
        let report = run_host_validate(&req);
        let p1 = report.waves.iter().find(|w| w.wave == "p1").unwrap();
        assert_eq!(p1.status, WaveStatus::Missing);
        assert_eq!(exit_code(&report), 78);
        assert!(p1.detail.contains("evidence NOT written"));
        assert!(!tmp.join("evidence").join("p1.json").exists());
    }

    #[test]
    fn explicit_operator_signature_is_passed_through() {
        let tmp = tempdir();
        let scripts = tmp.join("scripts");
        let evidence = tmp.join("evidence");
        std::fs::create_dir_all(&scripts).unwrap();
        std::fs::write(scripts.join("host-validate-verb-eval.sh"), "stub\n").unwrap();
        let req = ValidateRequest {
            mode: ValidateMode::Apply,
            evidence_dir: evidence.clone(),
            scripts_dir: scripts,
            only_wave: Some("p5".to_owned()),
            operator_signature: Some("operator:alice@laptop".to_owned()),
            timestamp_override: Some("2025-11-15T10:30:00Z".to_owned()),
            hostname_override: Some("test-host".to_owned()),
        };
        let _ = run_host_validate(&req);
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(evidence.join("p5.json")).unwrap())
                .unwrap();
        assert_eq!(v["operatorSignature"], "operator:alice@laptop");
    }

    // ----- in-tree tempdir helper (avoids adding `tempfile` dep) -----

    fn tempdir() -> PathBuf {
        let base = std::env::temp_dir();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pid = std::process::id();
        let path = base.join(format!("d2b-host-validate-test-{pid}-{nanos}"));
        std::fs::create_dir_all(&path).unwrap();
        path
    }
}
