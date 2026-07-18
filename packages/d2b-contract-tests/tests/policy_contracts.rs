//! Policy / doc-cross-reference / source-lint gates (the "H-group"), migrated
//! from the `tests/*.sh` bash gates. Each test reads the real repo files (via
//! the `d2b_contract_tests` repo-file helpers) and asserts a structural /
//! documentation invariant. This crate runs only from
//! `tests/tools/rust-workspace-checks.sh` against the real checkout (it is excluded
//! from the hermetic Nix sandbox workspace build), so repo-file access — and
//! the filesystem walk over `packages/**/*.rs` the tracing gate relies on — is
//! sound here.
//!
//! Migrated gates:
//!   * tests/tap-dag-contract-doc-eval.sh -> tap_dag_contract_doc_matches_implementation
//!   * tests/guest-exec-runtime-static.sh -> guest_exec_runtime_static
//!   * tests/tracing-contract-lint.sh     -> tracing_contract_lint

use std::collections::BTreeSet;
use std::fs;

use d2b_contract_tests::{read_repo_file, repo_path_exists, repo_root};
use regex::Regex;

/// Whether any single line of `content` matches `pattern`. This mirrors `grep`'s
/// (and ripgrep's) per-line evaluation faithfully — a `\s*` / `[[:space:]]*` in
/// the pattern can never span a newline boundary, as it could with a whole-file
/// `Regex::is_match`.
fn any_line_matches(content: &str, pattern: &str) -> bool {
    let re = Regex::new(pattern).expect("valid regex");
    content.lines().any(|line| re.is_match(line))
}

/// `grep -q PATTERN FILE` — assert at least one line of the repo-relative file
/// matches `pattern`.
fn assert_file_has_line(rel: &str, pattern: &str, ctx: &str) {
    assert!(
        any_line_matches(&read_repo_file(rel), pattern),
        "{ctx}: required pattern /{pattern}/ not found in {rel}"
    );
}

fn assert_any_file_has_line(files: &[&str], pattern: &str, ctx: &str) {
    let matched = files
        .iter()
        .filter(|rel| repo_path_exists(rel))
        .any(|rel| any_line_matches(&read_repo_file(rel), pattern));
    assert!(
        matched,
        "{ctx}: required pattern /{pattern}/ not found in {}",
        files.join(", ")
    );
}

/// `rg PATTERN FILES...` (fail-on-match) — assert NO line of any of the
/// repo-relative `files` matches `pattern`.
fn assert_files_have_no_line(files: &[&str], pattern: &str, ctx: &str) {
    let re = Regex::new(pattern).expect("valid regex");
    let mut hits: Vec<String> = Vec::new();
    for rel in files {
        let content = read_repo_file(rel);
        for (idx, line) in content.lines().enumerate() {
            if re.is_match(line) {
                hits.push(format!("{rel}:{}:{line}", idx + 1));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "{ctx}: forbidden pattern /{pattern}/ matched:\n{}",
        hits.join("\n")
    );
}

// ===========================================================================
// Migrated from tests/tap-dag-contract-doc-eval.sh.
//
// Doc/code drift gate. Asserts docs/reference/tap-dag-contract.md matches the
// implementation it documents: the derived-ifname scheme in
// d2b_host::ifname, the tap broker ops in d2b-priv-broker, the
// host-prep DAG variant + ordering edges in d2b_host::host_prep_dag, and
// the ChNetHandoffMode enum in d2b-core. Pure grep over committed sources;
// no nixpkgs eval, no rust build.
// ===========================================================================
#[test]
fn tap_dag_contract_doc_matches_implementation() {
    const DOC: &str = "docs/reference/tap-dag-contract.md";
    const IFNAME: &str = "packages/d2b-host/src/ifname.rs";
    const TAP_OPS: &str = "packages/d2b-priv-broker/src/ops/tap.rs";
    const DAG: &str = "packages/d2b-host/src/host_prep_dag.rs";
    const HOST_DTO: &str = "packages/d2b-core/src/host.rs";

    for f in [DOC, IFNAME, TAP_OPS, DAG, HOST_DTO] {
        assert!(
            repo_path_exists(f),
            "tap-dag-contract-doc-eval: missing {f}"
        );
    }

    let doc = read_repo_file(DOC);
    let ifname = read_repo_file(IFNAME);
    let tap_ops = read_repo_file(TAP_OPS);
    let dag = read_repo_file(DAG);
    let host_dto = read_repo_file(HOST_DTO);

    // ==> doc references existing source files.
    // Every relative `../../<path>` the doc points at must resolve. Spec
    // correction: this commit retires the bash gate scripts, so the doc's
    // own self-reference to `../../tests/tap-dag-contract-doc-eval.sh` (and
    // the sibling H-group scripts) is excluded here — the integrator sweeps
    // the doc cross-reference to the Rust successor. Keeping every other
    // referenced path's existence check intact preserves coverage.
    let retired_scripts: BTreeSet<&str> = [
        "tests/tap-dag-contract-doc-eval.sh",
        "tests/guest-exec-runtime-static.sh",
        "tests/tracing-contract-lint.sh",
    ]
    .into_iter()
    .collect();
    let doc_path_re = Regex::new(r"\.\./\.\./[a-zA-Z0-9._/-]+").expect("valid doc-path regex");
    let mut doc_paths: BTreeSet<String> = BTreeSet::new();
    for m in doc_path_re.find_iter(&doc) {
        // Strip the leading `../../`.
        doc_paths.insert(m.as_str()[6..].to_string());
    }
    for relpath in &doc_paths {
        if retired_scripts.contains(relpath.as_str()) {
            continue;
        }
        assert!(
            repo_path_exists(relpath),
            "tap-dag-contract-doc-eval: doc references missing path: {relpath}"
        );
    }

    // ==> ifname derivation contract.
    assert!(
        ifname.contains(r#"pub const DEFAULT_PREFIX: &str = "d2b-";"#),
        "tap-dag-contract-doc-eval: ifname.rs DEFAULT_PREFIX is not \"d2b-\""
    );
    assert!(
        doc.contains("`d2b-`"),
        "tap-dag-contract-doc-eval: doc must mention default prefix `d2b-`"
    );

    assert!(
        ifname.contains("pub const BRIDGE_TAG: char = 'b';"),
        "tap-dag-contract-doc-eval: ifname.rs BRIDGE_TAG is not 'b'"
    );
    assert!(
        ifname.contains("pub const TAP_TAG: char = 't';"),
        "tap-dag-contract-doc-eval: ifname.rs TAP_TAG is not 't'"
    );
    assert!(
        any_line_matches(&doc, "`t` for taps"),
        "tap-dag-contract-doc-eval: doc must document tap role tag 't'"
    );
    assert!(
        any_line_matches(&doc, "`b` for"),
        "tap-dag-contract-doc-eval: doc must document bridge role tag 'b'"
    );

    assert!(
        ifname.contains("pub const HASH_SUFFIX_LEN: usize = 8;"),
        "tap-dag-contract-doc-eval: ifname.rs HASH_SUFFIX_LEN is not 8"
    );
    assert!(
        any_line_matches(&doc, "(8 chars|HASH8|8-char)"),
        "tap-dag-contract-doc-eval: doc must document 8-char hash suffix"
    );

    assert!(
        doc.contains("derive_from_env_vm"),
        "tap-dag-contract-doc-eval: doc must reference derive_from_env_vm"
    );
    assert!(
        ifname.contains("pub fn derive_from_env_vm"),
        "tap-dag-contract-doc-eval: ifname.rs missing pub fn derive_from_env_vm"
    );

    assert!(
        doc.contains("looks_d2b_owned"),
        "tap-dag-contract-doc-eval: doc must reference looks_d2b_owned"
    );
    assert!(
        ifname.contains("pub fn looks_d2b_owned"),
        "tap-dag-contract-doc-eval: ifname.rs missing pub fn looks_d2b_owned"
    );

    // ==> tap broker ops contract.
    for op in ["CreateTapFd", "CreatePersistentTap", "SetBridgePortFlags"] {
        assert!(
            doc.contains(op),
            "tap-dag-contract-doc-eval: doc must mention broker op {op}"
        );
        assert!(
            tap_ops.contains(op),
            "tap-dag-contract-doc-eval: tap.rs missing {op}"
        );
    }

    // NM unmanaged pre-create gate.
    assert!(
        doc.contains("nm-unmanaged-pre-create-required"),
        "tap-dag-contract-doc-eval: doc must document nm-unmanaged-pre-create-required error"
    );
    assert!(
        tap_ops.contains("nm-unmanaged-pre-create-required"),
        "tap-dag-contract-doc-eval: tap.rs missing nm-unmanaged-pre-create-required error string"
    );

    // TUNSETPERSIST / TUNSETOWNER / TUNSETGROUP for persistent mode.
    for sym in ["TUNSETPERSIST", "TUNSETOWNER", "TUNSETGROUP"] {
        assert!(
            doc.contains(sym),
            "tap-dag-contract-doc-eval: doc must document {sym}"
        );
    }
    assert!(
        tap_ops.contains("TUNSETOWNER"),
        "tap-dag-contract-doc-eval: tap.rs missing TUNSETOWNER reference"
    );

    // ==> host-prep DAG step + ordering.
    assert!(
        doc.contains("BringUpTapInterface"),
        "tap-dag-contract-doc-eval: doc must reference BringUpTapInterface step"
    );
    assert!(
        any_line_matches(&dag, r"^\s+BringUpTapInterface,?$"),
        "tap-dag-contract-doc-eval: host_prep_dag.rs missing BringUpTapInterface variant"
    );

    assert!(
        any_line_matches(&dag, r#""bring-up-tap-interface""#),
        "tap-dag-contract-doc-eval: host_prep_dag.rs missing bring-up-tap-interface step_id slug"
    );
    assert!(
        doc.contains("bring-up-tap-interface"),
        "tap-dag-contract-doc-eval: doc must reference bring-up-tap-interface step_id slug"
    );

    // Documented broker op name for the step.
    assert!(
        any_line_matches(&dag, r#"Self::BringUpTapInterface\s*=>\s*"CreateTapFd""#),
        "tap-dag-contract-doc-eval: host_prep_dag.rs BringUpTapInterface.broker_op_name must be \
         \"CreateTapFd\""
    );

    // Ordering edges: apply-nftables-rules -> bring-up -> pre-open-vhost-net-fd.
    assert!(
        doc.contains("apply-nftables-rules"),
        "tap-dag-contract-doc-eval: doc must document upstream gate apply-nftables-rules"
    );
    assert!(
        doc.contains("pre-open-vhost-net-fd"),
        "tap-dag-contract-doc-eval: doc must document downstream consumer pre-open-vhost-net-fd"
    );

    // Failure envelope.
    assert!(
        doc.contains("HostPrepStepFailed"),
        "tap-dag-contract-doc-eval: doc must reference HostPrepStepFailed"
    );
    assert!(
        dag.contains("pub struct HostPrepStepFailed"),
        "tap-dag-contract-doc-eval: host_prep_dag.rs missing HostPrepStepFailed"
    );

    // ==> ChNetHandoffMode enum.
    for variant in ["TapFd", "PersistentTap"] {
        assert!(
            doc.contains(variant),
            "tap-dag-contract-doc-eval: doc must document ChNetHandoffMode::{variant}"
        );
        assert!(
            any_line_matches(&host_dto, &format!(r"^\s+{variant},?$|^\s+{variant}\b")),
            "tap-dag-contract-doc-eval: host.rs missing ChNetHandoffMode::{variant}"
        );
    }
    assert!(
        host_dto.contains("pub enum ChNetHandoffMode"),
        "tap-dag-contract-doc-eval: host.rs missing pub enum ChNetHandoffMode"
    );
    assert!(
        doc.contains("ChNetHandoffMode"),
        "tap-dag-contract-doc-eval: doc must reference ChNetHandoffMode"
    );

    // ==> launcher group naming (daemon-only canonical). The doc claims the
    // broker public socket sits behind the daemon-only `d2b` group
    // declared by host-daemon.nix; sanity check that's still true.
    assert!(
        doc.contains("d2b"),
        "tap-dag-contract-doc-eval: doc must reference daemon-only d2b group"
    );
    assert!(
        read_repo_file("nixos-modules/host-daemon.nix").contains("users.groups.d2b"),
        "tap-dag-contract-doc-eval: host-daemon.nix no longer declares d2b group"
    );
}

// ===========================================================================
// Migrated from tests/guest-exec-runtime-static.sh.
//
// Guest exec runtime static guard. Asserts:
//   * the ATTACHED non-interactive exec runtime stays inside its scope —
//     guestd-local process execution only, no userd call path, no low-level
//     TTY/PTY syscalls, no detached retained-log writes in the attached path,
//     stdin closed (never piped), no extra vsock listeners, no CH
//     CONNECT/relay/host-network/observability surface;
//   * the DETACHED path is present-and-bounded — slot-keyed transient units
//     (no opaque exec id in unit name/argv), scoped to d2b-exec.slice,
//     truncation-bounded retained logs, conditionally-advertised capabilities,
//     and the guest-module-declared parent dir + slice;
//   * the INTERACTIVE TTY path is present-and-confined — PTY master allocation
//     lives only in exec_pty.rs, the setsid + TIOCSCTTY controlling-terminal
//     handshake lives ONLY in the exec-runner --tty-exec helper (guestd never
//     acquires a controlling tty), and the typed stderr-unavailable wire
//     mapping is wired in service.rs.
// ===========================================================================
#[test]
fn guest_exec_runtime_static() {
    const GUESTD_SRC: &str = "packages/d2b-guestd/src";
    let exec_src = format!("{GUESTD_SRC}/exec.rs");
    let exec_linux_src = format!("{GUESTD_SRC}/exec_linux.rs");
    let service_src = format!("{GUESTD_SRC}/service.rs");
    let production_guest_src = format!("{GUESTD_SRC}/production_guest.rs");

    // The runtime must exist (this guard is meaningless otherwise).
    for required in [&exec_src, &exec_linux_src] {
        assert!(
            repo_path_exists(required),
            "guest-exec-runtime-static: missing {required}"
        );
    }
    let exec_pair: &[&str] = &[&exec_src, &exec_linux_src];

    // No userd runtime call path in the exec runtime.
    assert_files_have_no_line(
        exec_pair,
        r"userd|d2b-userd",
        "guest-exec-runtime-static: exec runtime must not reference userd",
    );

    // No LOW-LEVEL TTY/PTY syscalls in the ATTACHED exec runtime. The
    // interactive PTY mechanism lives entirely in exec_pty.rs (the guestd-side
    // spawner) and the exec-runner `--tty-exec` helper; exec.rs/exec_linux.rs
    // may reference the spawner *type names* but must never allocate a PTY or
    // perform the controlling-terminal handshake themselves. The `\bsetsid\(` /
    // `openpt\(` call forms keep prose mentions from tripping the guard.
    assert_files_have_no_line(
        exec_pair,
        r"openpty|forkpty|login_tty|set_controlling|openpt\(|grantpt\(|unlockpt\(|ptsname\(|ioctl_tiocsctty|\bsetsid\(",
        "guest-exec-runtime-static: attached exec must not perform low-level TTY/PTY syscalls \
         (the PTY mechanism lives in exec_pty.rs + the --tty-exec helper)",
    );

    // No detached retained-log file writes from the ATTACHED exec runtime.
    assert_files_have_no_line(
        exec_pair,
        r"File::create|OpenOptions|fs::write",
        "guest-exec-runtime-static: attached exec runtime must not write retained log files",
    );

    // stdin must be closed (redirected to /dev/null), never piped/open.
    assert_file_has_line(
        &exec_linux_src,
        r"Stdio::null",
        "guest-exec-runtime-static: spawned children must redirect stdin to /dev/null",
    );
    assert_files_have_no_line(
        &[&exec_linux_src],
        r"stdin\(Stdio::piped",
        "guest-exec-runtime-static: spawned children must not pipe stdin",
    );

    // The only vsock listener lives in the service transport, not the exec
    // runtime.
    assert_files_have_no_line(
        exec_pair,
        r"VsockListener|VsockAddr",
        "guest-exec-runtime-static: exec runtime must not open its own vsock listener",
    );

    // No CH CONNECT / relay / host firewall / observability surface in the
    // runtime.
    assert_files_have_no_line(
        exec_pair,
        r"CONNECT|nftables|iptables|/etc/hosts|otel|exporter|prometheus",
        "guest-exec-runtime-static: exec runtime must not touch host \
         network/observability surfaces",
    );

    // --- Detached path: present-and-bounded, not absent. ---
    let detached_registry_src = format!("{GUESTD_SRC}/detached_registry.rs");
    let detached_unit_src = format!("{GUESTD_SRC}/detached.rs");
    const RUNNER_SRC: &str = "packages/d2b-exec-runner/src";

    for required in [&detached_registry_src, &detached_unit_src] {
        assert!(
            repo_path_exists(required),
            "guest-exec-runtime-static: missing detached source {required}"
        );
    }
    assert!(
        repo_path_exists(RUNNER_SRC),
        "guest-exec-runtime-static: missing exec-runner source dir {RUNNER_SRC}"
    );

    // Transient units are slot-keyed and carry no opaque exec id in the unit
    // name. `d2b-exec-<NN>.service` is the only allowed shape.
    assert_file_has_line(
        &detached_unit_src,
        r"d2b-exec-\{slot",
        "guest-exec-runtime-static: detached units must be slot-keyed (d2b-exec-<NN>)",
    );

    // The opaque exec id must NEVER appear in the unit name or systemd-run argv.
    // (It is confined to the spec/status files under the slot dir.) Scope this
    // to production code: the test module legitimately asserts the *absence* of
    // the token, which would otherwise be a false positive.
    let detached_unit = read_repo_file(&detached_unit_src);
    let cfg_test_re = Regex::new(r"^#\[cfg\(test\)\]").expect("valid cfg(test) regex");
    let detached_unit_prod: String = detached_unit
        .lines()
        .take_while(|line| !cfg_test_re.is_match(line))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !detached_unit_prod.contains("exec_id"),
        "guest-exec-runtime-static: opaque exec id must not appear in unit name/argv"
    );

    // Detached transient units are scoped to the dedicated guest-internal slice.
    assert_file_has_line(
        &detached_unit_src,
        r"d2b-exec\.slice",
        "guest-exec-runtime-static: detached units must be scoped to d2b-exec.slice",
    );

    // The retained-log path is truncation-bounded (drop-oldest accounting).
    assert_file_has_line(
        "packages/d2b-exec-runner/src/filering.rs",
        r"truncated|dropped",
        "guest-exec-runtime-static: detached retained logs must be truncation-bounded",
    );

    // Capabilities are advertised conditionally (usability-aware), not
    // always-on.
    assert_any_file_has_line(
        &[&service_src, &production_guest_src],
        r"EXEC_DETACHED|EXEC_LOGS",
        "guest-exec-runtime-static: detached/logs capabilities must be wired conditionally",
    );

    // The detached parent dir + slice are declared in the guest module.
    assert_file_has_line(
        "nixos-modules/guest-control.nix",
        r"/run/d2b-exec",
        "guest-exec-runtime-static: guest module must declare /run/d2b-exec parent dir",
    );
    assert_file_has_line(
        "nixos-modules/guest-control.nix",
        r"d2b-exec",
        "guest-exec-runtime-static: guest module must declare the d2b-exec slice",
    );

    // --- Interactive TTY path: present-and-confined, not absent. ---
    let exec_pty_src = format!("{GUESTD_SRC}/exec_pty.rs");
    let tty_helper_src = format!("{RUNNER_SRC}/tty_helper.rs");

    for required in [&exec_pty_src, &tty_helper_src] {
        assert!(
            repo_path_exists(required),
            "guest-exec-runtime-static: missing interactive TTY source {required}"
        );
    }

    // The guestd-side PTY spawner owns master allocation (openpt/grantpt/
    // unlockpt/ptsname). Confining it to exec_pty.rs is what keeps exec.rs
    // PTY-syscall-free.
    assert_file_has_line(
        &exec_pty_src,
        r"openpt\(",
        "guest-exec-runtime-static: exec_pty.rs must own PTY master allocation (openpt)",
    );

    // The controlling-terminal handshake (setsid + TIOCSCTTY) lives ONLY in the
    // static --tty-exec helper. This is the no-first-party-unsafe crux:
    // guestd never acquires a controlling tty.
    assert_file_has_line(
        &tty_helper_src,
        r"\bsetsid\b",
        "guest-exec-runtime-static: --tty-exec helper must perform the setsid + TIOCSCTTY handshake",
    );
    assert_file_has_line(
        &tty_helper_src,
        r"ioctl_tiocsctty",
        "guest-exec-runtime-static: --tty-exec helper must perform the setsid + TIOCSCTTY handshake",
    );

    // guestd (exec_pty.rs) must NOT perform that handshake itself. Strip line
    // comments first so the design-rationale prose that *names* setsid/TIOCSCTTY
    // does not trip the guard.
    let exec_pty = read_repo_file(&exec_pty_src);
    let line_comment_re = Regex::new(r"^\s*//").expect("valid line-comment regex");
    let handshake_re = Regex::new(r"ioctl_tiocsctty|\bsetsid\b").expect("valid handshake regex");
    let exec_pty_offenders: Vec<String> = exec_pty
        .lines()
        .enumerate()
        .filter(|(_, line)| !line_comment_re.is_match(line))
        .filter(|(_, line)| handshake_re.is_match(line))
        .map(|(idx, line)| format!("{exec_pty_src}:{}:{line}", idx + 1))
        .collect();
    assert!(
        exec_pty_offenders.is_empty(),
        "guest-exec-runtime-static: guestd must not perform the controlling-terminal handshake \
         (it routes through the --tty-exec helper):\n{}",
        exec_pty_offenders.join("\n")
    );

    // The TTY merged-output contract surfaces a typed stderr-unavailable error;
    // the wire mapping must be wired in the service layer.
    assert_any_file_has_line(
        &[&service_src, &exec_src],
        r"TtyStderrUnavailable",
        "guest-exec-runtime-static: TTY stderr-unavailable wire mapping missing",
    );
}

// ===========================================================================
// Migrated from tests/tracing-contract-lint.sh.
//
// Static enforcement of the bounded-cardinality tracing-attribute allowlist
// documented in docs/reference/tracing-contract.md. Walks workspace Rust
// source (packages/**/*.rs, excluding generated / vendored / target trees,
// INCLUDING tests because integration tests can regress the contract too) and
// fails closed if any historically-forbidden high-cardinality / leakable
// attribute shape appears. Per-VM bounded path attributes are tolerated; this
// gate refuses only the bundle / store-path / argv / secret / child-output
// classes.
// ===========================================================================
#[test]
fn tracing_contract_lint() {
    let rust_files = collect_workspace_rust_files();
    assert!(
        !rust_files.is_empty(),
        "tracing-contract-lint: no Rust source files found under packages/ — wrong CWD?"
    );

    let mut violations: Vec<String> = Vec::new();

    // -- Single-line forbidden attribute classes ----------------------------
    // (description, ERE pattern) — verbatim ports of the bash gate's `scan`
    // calls. Descriptions are phrased to avoid the `<name> = %/?` shapes that
    // would make this very file a self-violation when the gate scans it.
    let scans: &[(&str, &str)] = &[
        (
            "bundle .display() path attr (forbidden high-cardinality store path)",
            r"bundle[[:space:]]*=[[:space:]]*[%?][^,]*\.display\(\)",
        ),
        (
            "bundle_path attr (alias of forbidden bundle attr)",
            r"bundle_path[[:space:]]*=[[:space:]]*[%?]",
        ),
        (
            "keys_dir .display() attr (surface via outcome + audit instead)",
            r"keys_dir[[:space:]]*=[[:space:]]*[%?][^,]*\.display\(\)",
        ),
        (
            "argv attr in tracing (forbidden — operator-supplied content; route via typed envelope)",
            r"(^|[^_a-zA-Z0-9])argv[[:space:]]*=[[:space:]]*[%?]",
        ),
        (
            "cmdline attr in tracing (forbidden — see argv rule)",
            r"(^|[^_a-zA-Z0-9])cmdline[[:space:]]*=[[:space:]]*[%?]",
        ),
        (
            "command_line attr in tracing (forbidden — see argv rule)",
            r"(^|[^_a-zA-Z0-9])command_line[[:space:]]*=[[:space:]]*[%?]",
        ),
        (
            "process_env attr in tracing (forbidden — environment leak)",
            r"(^|[^_a-zA-Z0-9])process_env[[:space:]]*=[[:space:]]*[%?]",
        ),
        (
            "environment attr in tracing (forbidden — environment leak)",
            r"(^|[^_a-zA-Z0-9])environment[[:space:]]*=[[:space:]]*[%?]",
        ),
        (
            "cwd attr in tracing (forbidden — working-directory leak)",
            r"(^|[^_a-zA-Z0-9])cwd[[:space:]]*=[[:space:]]*[%?]",
        ),
        (
            "current_working_directory attr in tracing (forbidden — working-directory leak)",
            r"(^|[^_a-zA-Z0-9])current_working_directory[[:space:]]*=[[:space:]]*[%?]",
        ),
        (
            "secret attr in tracing (forbidden — credential leak)",
            r"(^|[^_a-zA-Z0-9])secret[[:space:]]*=[[:space:]]*[%?]",
        ),
        (
            "password attr in tracing (forbidden — credential leak)",
            r"(^|[^_a-zA-Z0-9])password[[:space:]]*=[[:space:]]*[%?]",
        ),
        (
            "token attr in tracing (forbidden — credential leak)",
            r"(^|[^_a-zA-Z0-9])token[[:space:]]*=[[:space:]]*[%?]",
        ),
        (
            "private_key attr in tracing (forbidden — credential leak)",
            r"(^|[^_a-zA-Z0-9])private_key[[:space:]]*=[[:space:]]*[%?]",
        ),
        (
            "provider attr in tracing (forbidden — provider context leak)",
            r"(^|[^_a-zA-Z0-9])provider[[:space:]]*=[[:space:]]*[%?]",
        ),
        (
            "provider_endpoint attr in tracing (forbidden — provider endpoint leak)",
            r"(^|[^_a-zA-Z0-9])provider_endpoint[[:space:]]*=[[:space:]]*[%?]",
        ),
        (
            "provider_resource_id attr in tracing (forbidden — provider resource leak)",
            r"(^|[^_a-zA-Z0-9])provider_resource_id[[:space:]]*=[[:space:]]*[%?]",
        ),
        (
            "provider_credential attr in tracing (forbidden — provider credential leak)",
            r"(^|[^_a-zA-Z0-9])provider_credential[[:space:]]*=[[:space:]]*[%?]",
        ),
        (
            "credential attr in tracing (forbidden — credential leak)",
            r"(^|[^_a-zA-Z0-9])credential[[:space:]]*=[[:space:]]*[%?]",
        ),
        (
            "stream attr in tracing (forbidden — terminal stream context leak)",
            r"(^|[^_a-zA-Z0-9])stream[[:space:]]*=[[:space:]]*[%?]",
        ),
        (
            "stream_id attr in tracing (forbidden — terminal stream id leak)",
            r"(^|[^_a-zA-Z0-9])stream_id[[:space:]]*=[[:space:]]*[%?]",
        ),
        (
            "terminal_stream_id attr in tracing (forbidden — terminal stream id leak)",
            r"(^|[^_a-zA-Z0-9])terminal_stream_id[[:space:]]*=[[:space:]]*[%?]",
        ),
        (
            "attach_id attr in tracing (forbidden — terminal attach id leak)",
            r"(^|[^_a-zA-Z0-9])attach_id[[:space:]]*=[[:space:]]*[%?]",
        ),
        (
            "session attr in tracing (forbidden — terminal session context leak)",
            r"(^|[^_a-zA-Z0-9])session[[:space:]]*=[[:space:]]*[%?]",
        ),
        (
            "session_id attr in tracing (forbidden — terminal session id leak)",
            r"(^|[^_a-zA-Z0-9])session_id[[:space:]]*=[[:space:]]*[%?]",
        ),
        (
            "session-id attr in tracing (forbidden — terminal session id leak)",
            r"(^|[^_a-zA-Z0-9])session-id[[:space:]]*=[[:space:]]*[%?]",
        ),
        (
            "resource attr in tracing (forbidden — provider resource leak)",
            r"(^|[^_a-zA-Z0-9])resource[[:space:]]*=[[:space:]]*[%?]",
        ),
        (
            "resource_id attr in tracing (forbidden — provider resource leak)",
            r"(^|[^_a-zA-Z0-9])resource_id[[:space:]]*=[[:space:]]*[%?]",
        ),
        (
            "stdout attr in tracing (forbidden — child output; route via typed envelope)",
            r"(^|[^_a-zA-Z0-9])stdout[[:space:]]*=[[:space:]]*[%?]",
        ),
        (
            "stderr attr in tracing (forbidden — child output; route via typed envelope)",
            r"(^|[^_a-zA-Z0-9])stderr[[:space:]]*=[[:space:]]*[%?]",
        ),
    ];

    for (description, pattern) in scans {
        let re = Regex::new(pattern).expect("valid scan regex");
        let mut hits: Vec<String> = Vec::new();
        for rel in &rust_files {
            let content = read_repo_file(rel);
            for (idx, line) in content.lines().enumerate() {
                if re.is_match(line) {
                    hits.push(format!("{rel}:{}:{line}", idx + 1));
                }
            }
        }
        if !hits.is_empty() {
            violations.push(format!("VIOLATION: {description}\n{}", hits.join("\n")));
        }
    }

    // -- store-path literal strings inside tracing arg lists ----------------
    // The forbidden-literal regex is assembled from fragments so this very
    // source file does not embed the contiguous literal it scans for (which
    // would otherwise self-match when the state machine runs over this file).
    let store_lit_pattern = ["\"", "/nix", "/store/", "[^\"]+", "\""].concat();
    let store_lit = Regex::new(&store_lit_pattern).expect("valid store-literal regex");
    let comment_line_re = Regex::new(r"^[[:space:]]*//").expect("valid comment-line regex");

    // First: are there ANY non-comment store-path literals at all? (Mirrors the
    // bash gate's `nix_store_hits` gate — the awk state machine only runs when
    // at least one non-comment literal exists.)
    let mut store_literals_present = false;
    'outer: for rel in &rust_files {
        let content = read_repo_file(rel);
        for line in content.lines() {
            if store_lit.is_match(line) && !comment_line_re.is_match(line) {
                store_literals_present = true;
                break 'outer;
            }
        }
    }

    if store_literals_present {
        // Faithful port of the bash gate's awk state machine: track whether the
        // current line sits inside a tracing-macro argument list (via paren
        // depth) and flag any store-path literal that does. `in_tracing` /
        // `depth` persist across files exactly as the awk globals do.
        let tracing_start = Regex::new(
            r"tracing::(info|warn|error|debug|trace|event|span)!|^[[:space:]]*(info|warn|error|debug|trace)!\(",
        )
        .expect("valid tracing-start regex");
        let mut in_tracing = false;
        let mut depth: i64 = 0;
        let mut bad: Vec<String> = Vec::new();
        for rel in &rust_files {
            let content = read_repo_file(rel);
            for (idx, line) in content.lines().enumerate() {
                if tracing_start.is_match(line) {
                    in_tracing = true;
                    depth = 0;
                }
                if in_tracing {
                    for c in line.chars() {
                        if c == '(' {
                            depth += 1;
                        } else if c == ')' {
                            depth -= 1;
                            if depth <= 0 {
                                in_tracing = false;
                                break;
                            }
                        }
                    }
                    if store_lit.is_match(line) {
                        bad.push(format!("{rel}:{}:{line}", idx + 1));
                    }
                }
            }
        }
        if !bad.is_empty() {
            violations.push(format!(
                "VIOLATION: store-path literal inside a tracing macro arg list\n{}",
                bad.join("\n")
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "tracing-contract-lint: {} forbidden high-cardinality / leakable tracing attr class(es) \
         detected — see docs/reference/tracing-contract.md\n\n{}",
        violations.len(),
        violations.join("\n\n")
    );
}

/// Enumerate repo-relative `packages/**/*.rs` files, excluding `target/`,
/// `vendor/`, and `generated/` trees. Mirrors the bash gate's
/// `find packages -type f -name '*.rs' -not -path '*/target/*' ...` (a plain
/// filesystem walk that does not follow symlinks), returning sorted paths.
fn collect_workspace_rust_files() -> Vec<String> {
    let root = repo_root();
    let mut out: Vec<String> = Vec::new();
    walk_rust_files(&root.join("packages"), &root, &mut out);
    out.retain(|p| {
        !p.contains("/target/") && !p.contains("/vendor/") && !p.contains("/generated/")
    });
    out.sort();
    out
}

fn walk_rust_files(dir: &std::path::Path, root: &std::path::Path, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        // Do not follow symlinks (matches `find`'s default), and prune the
        // excluded trees so the walk stays fast.
        if file_type.is_dir() {
            match entry.file_name().to_str() {
                Some("target") | Some("vendor") | Some("generated") | Some(".git") => continue,
                _ => walk_rust_files(&path, root, out),
            }
        } else if file_type.is_file()
            && path.extension().is_some_and(|ext| ext == "rs")
            && let Ok(rel) = path.strip_prefix(root)
        {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}
