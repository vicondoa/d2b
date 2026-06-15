//! ADR 0015 fail-closed source-lint gate (the "H-group"), migrated from the
//! `tests/legacy-unit-denylist-eval.sh` bash gate. The test reads the real
//! `nixos-modules/` tree and asserts the daemon-only clean break: no systemd
//! unit name retired by the pre-daemon supervisor may reappear as live wiring.
//! This crate runs only from `tests/rust-workspace-checks.sh` against the real
//! checkout (it is excluded from the hermetic Nix sandbox workspace build), so
//! repo-file access — and shelling out to `git` for the file enumeration the
//! bash gate got from `find` — is sound here.
//!
//! Migrated gate:
//!   * tests/legacy-unit-denylist-eval.sh -> legacy_unit_denylist
//!
//! Self-flag note: the denylist needles this file carries (e.g.
//! `nixling-${name}-gpu`) could match this very file — but the gate's file
//! enumeration is scoped to `nixos-modules/` only (never `packages/` or
//! `tests/`), so this port under `packages/` is never scanned by its own
//! denylist and needs no self-allowlist. None of the needles contain the
//! literal legacy launcher group name, so the sibling `legacy_group_name_denylist`
//! gate (which scans `packages/`) is not tripped either.

use std::collections::BTreeSet;
use std::process::Command;

use nixling_contract_tests::repo_root;
use regex::Regex;

/// Read a repo-relative file, returning `None` when the path is absent or not
/// valid UTF-8 (binary files are skipped, mirroring `grep -I`).
fn read_repo_file_opt(rel: &str) -> Option<String> {
    std::fs::read_to_string(repo_root().join(rel)).ok()
}

/// Enumerate repo-relative tracked + untracked-non-ignored files under the given
/// pathspecs via `git ls-files`. The bash gate used `find "$MODULES_DIR" -type f`
/// (which ignores `.gitignore`); for the `nixos-modules/` source tree — which
/// carries no build artifacts — `git ls-files --cached --others
/// --exclude-standard` enumerates the identical set, and is the convention every
/// sibling `policy_*.rs` port already uses.
fn git_listed_files(roots: &[&str]) -> Vec<String> {
    let root = repo_root();
    let output = Command::new("git")
        .arg("-C")
        .arg(&root)
        .arg("-c")
        .arg("core.quotePath=false")
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
        ])
        .args(roots)
        .output()
        .expect("run `git ls-files`");
    assert!(
        output.status.success(),
        "git ls-files failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut files: BTreeSet<String> = BTreeSet::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if !line.is_empty() {
            files.insert(line.to_string());
        }
    }
    files.into_iter().collect()
}

/// The `<vm>` / `<env>` placeholder from the bash gate's `NAMEPART`: a literal
/// name segment (`[A-Za-z0-9_-]+`) OR a Nix string-interpolation token
/// (`${...}`), so both `"nixling-foo-snd"` and `"nixling-${m.name}-snd"` match.
const NAMEPART: &str = r"([A-Za-z0-9_-]+|\$\{[^}]+\})";

/// Verbatim port of the bash gate's `PATTERNS=(...)` extended-regex denylist.
/// Returns `(raw, compiled)` pairs so a violation can name the offending
/// pattern, exactly as the bash `printf '... pattern=%s ...'` did.
fn denylist_patterns() -> Vec<(String, Regex)> {
    let raw = vec![
        r"microvm-tap-interfaces@".to_string(),
        r"microvm-setup@".to_string(),
        format!("nixling-{NAMEPART}-snd"),
        format!("nixling-{NAMEPART}-video"),
        format!("nixling-{NAMEPART}-gpu"),
        format!("nixling-{NAMEPART}-store-sync"),
        r"nixling-known-hosts-refresh@".to_string(),
        r"nixling-otel-relay@".to_string(),
        r"nixling-net-route-preflight".to_string(),
        r"nixling-audit-check\.service".to_string(),
        r"nixling-audit-check\.timer".to_string(),
        r"nixling-ch-exporter".to_string(),
        r"nixling-otel-host-bridge\.service".to_string(),
        format!("nixling-sys-{NAMEPART}-usbipd-"),
    ];
    raw.into_iter()
        .map(|p| {
            let re =
                Regex::new(&p).unwrap_or_else(|e| panic!("invalid denylist pattern {p:?}: {e}"));
            (p, re)
        })
        .collect()
}

/// The files the bash gate scanned: every `*.nix` / `*.md` under
/// `nixos-modules/`, sorted. `git_listed_files` already returns a sorted set.
fn module_files() -> Vec<String> {
    let files: Vec<String> = git_listed_files(&["nixos-modules"])
        .into_iter()
        .filter(|rel| rel.ends_with(".nix") || rel.ends_with(".md"))
        .collect();
    assert!(
        !files.is_empty(),
        "legacy-unit-denylist: no *.nix/*.md files found under nixos-modules/ \
         (expected the framework module tree)"
    );
    files
}

#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Skip,
    Live,
}

/// Whether `line` contains every part of `parts` in left-to-right order, mirroring
/// a shell `case` glob of the shape `*p0*p1*...*` (each `*` may span any chars,
/// including `/`). Used for the observability/host.nix journald-filter carve-out.
fn ordered_contains(line: &str, parts: &[&str]) -> bool {
    let mut rest = line;
    for part in parts {
        match rest.find(part) {
            Some(idx) => rest = &rest[idx + part.len()..],
            None => return false,
        }
    }
    true
}

/// Faithful port of the bash gate's `classify()`: decide whether a matched line
/// is a `Live` legacy-unit reference (a failure) or a `Skip` (comment, retirement
/// marker, per-file allowlist, or content-based carve-out).
///
/// `lines[idx]` is the matched line; `idx` is 0-based (the bash `lineno` is
/// `idx + 1`). The preceding-line marker check reads `lines[idx - 1]`.
fn classify(rel: &str, idx: usize, lines: &[String]) -> Verdict {
    let line = &lines[idx];

    // Pure comment? (line, after leading whitespace, begins with `#`).
    if line.trim_start().starts_with('#') {
        return Verdict::Skip;
    }

    // Inline retirement marker on the matched line itself.
    if line.contains("# obituary:") || line.contains("# retired:") {
        return Verdict::Skip;
    }

    // Retirement marker on the immediately preceding line (lineno > 1).
    if idx > 0 {
        let prev = &lines[idx - 1];
        if prev.contains("# obituary:") || prev.contains("# retired:") {
            return Verdict::Skip;
        }
    }

    // Per-file allowlist for legitimate non-unit-declaration contexts. The bash
    // `case` globs use `*` which spans `/`; these checks reproduce that.
    if rel.contains("/components/") && rel.ends_with("/guest.nix") {
        // Guest-side scripts (run inside the VM, not on the host).
        return Verdict::Skip;
    }
    if rel.ends_with(".md") {
        // Markdown docstrings; the doc-drift gate covers those.
        return Verdict::Skip;
    }
    if rel.ends_with("/host-users.nix") {
        // Declares user/group names — NOT systemd units.
        return Verdict::Skip;
    }
    if rel.ends_with("/minijail-profiles.nix") {
        // Uses the principal name for setresuid().
        return Verdict::Skip;
    }
    if rel.ends_with("/manifest.nix") {
        // Bundle metadata strings the broker consumes — NOT unit declarations.
        return Verdict::Skip;
    }
    if rel.ends_with("/processes-json.nix") {
        // Bundle processes taxonomy identifier strings — NOT unit declarations.
        return Verdict::Skip;
    }
    if rel.ends_with("/components/observability/host.nix") {
        // Alloy journald source filters may name the historical units; only an
        // actual `systemd.services`/`systemd.sockets` declaration is a failure.
        if ordered_contains(line, &["systemd.services.\"", "\" = {"])
            || ordered_contains(line, &["systemd.services.", " = {"])
            || ordered_contains(line, &["systemd.sockets.", " = {"])
        {
            return Verdict::Live;
        }
        return Verdict::Skip;
    }
    if rel.ends_with("/components/observability/stack.nix") {
        // Prometheus alert-rule regex pinning the legacy name — historical.
        return Verdict::Skip;
    }
    if rel.ends_with("/host-activation.nix") {
        // Transitional setfacl + `systemctl is-active` no-op-when-absent checks.
        return Verdict::Skip;
    }
    if rel.ends_with("/assertions.nix") {
        // Operator-facing remediation prose — NOT declarations.
        return Verdict::Skip;
    }
    if rel.ends_with("/components/audio/host.nix") {
        // Transitional setfacl helpers + docstrings, no-op when legacy user absent.
        return Verdict::Skip;
    }

    // Inline content-based skip for the few remaining contexts without a
    // dedicated file allowlist: bundle-metadata `*Service = "nixling-..."` strings.
    if line.contains("audioService = \"nixling-")
        || line.contains("videoService = \"nixling-")
        || line.contains("gpuService = \"nixling-")
        || line.contains("tpmService = \"nixling-")
    {
        return Verdict::Skip;
    }

    Verdict::Live
}

// ---------------------------------------------------------------------------
// Migrated from tests/legacy-unit-denylist-eval.sh (ADR 0015).
//
// No systemd unit name retired in the daemon-only clean break may reappear as
// live wiring under nixos-modules/. "Live wiring" = any denylist-pattern match
// that is not a pure comment, not tagged with an inline/preceding-line
// `# obituary:` / `# retired:` marker, and not in the per-file / content-based
// allowlist. The gate scans every *.nix and *.md file under nixos-modules/.
// ---------------------------------------------------------------------------
#[test]
fn legacy_unit_denylist() {
    let patterns = denylist_patterns();

    // Pre-load each file's lines once; classify() needs the preceding line.
    let files: Vec<(String, Vec<String>)> = module_files()
        .into_iter()
        .filter_map(|rel| {
            read_repo_file_opt(&rel).map(|content| {
                let lines: Vec<String> = content.lines().map(str::to_string).collect();
                (rel, lines)
            })
        })
        .collect();

    // Iterate per pattern, then per file, then per line — mirroring the bash
    // `for pattern; do grep -HnE ...; done` so each pattern is evaluated
    // independently against every line.
    let mut violations: Vec<String> = Vec::new();
    for (raw, re) in &patterns {
        for (rel, lines) in &files {
            for (idx, line) in lines.iter().enumerate() {
                if !re.is_match(line) {
                    continue;
                }
                if classify(rel, idx, lines) == Verdict::Live {
                    violations.push(format!("pattern={raw}\n  {rel}:{}: {line}", idx + 1));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "legacy-unit-denylist: {} live legacy-unit reference(s) in nixos-modules/ \
         (ADR 0015 — retired per-VM systemd templates and host-singleton \
         framework services must not reappear as live wiring):\n{}",
        violations.len(),
        violations.join("\n")
    );
}

/// Negative coverage for `classify()`: each retirement / allowlist escape hatch
/// must skip, while a genuine live unit declaration in a non-exempt module must
/// be flagged. This guards the fail-closed posture against silent loosening.
#[test]
fn legacy_unit_denylist_classify_semantics() {
    // Pure comment → skip.
    assert_eq!(
        classify(
            "nixos-modules/network.nix",
            0,
            &["    # nixling-net-route-preflight.service was deleted".to_string()]
        ),
        Verdict::Skip,
        "a pure comment naming a retired unit must skip"
    );

    // Inline retirement marker on a code line → skip.
    assert_eq!(
        classify(
            "nixos-modules/host.nix",
            0,
            &[r#"    services.foo = "nixling-bar-gpu"; # retired: transitional"#.to_string()]
        ),
        Verdict::Skip,
        "an inline `# retired:` marker must skip"
    );

    // Preceding-line retirement marker → skip.
    assert_eq!(
        classify(
            "nixos-modules/host-otel-relay-acl.nix",
            1,
            &[
                "      # retired: ch-exporter ACL — remnant of the deleted nixling-ch-exporter.service"
                    .to_string(),
                r#"      refresh_acl_set "g:nixling-ch-exporter" dirs socks"#.to_string(),
            ]
        ),
        Verdict::Skip,
        "a `# retired:` marker on the preceding line must skip the code line"
    );

    // Per-file allowlist (manifest.nix carries bundle-metadata service strings).
    assert_eq!(
        classify(
            "nixos-modules/manifest.nix",
            0,
            &[r#"    gpuService = "nixling-${m.name}-gpu";"#.to_string()]
        ),
        Verdict::Skip,
        "manifest.nix bundle-metadata strings are exempt"
    );

    // observability/host.nix: a journald-filter string is skipped...
    assert_eq!(
        classify(
            "nixos-modules/components/observability/host.nix",
            0,
            &[r#"        unit_pattern = "nixling-otel-relay@*.service";"#.to_string()]
        ),
        Verdict::Skip,
        "observability journald-filter strings naming a retired unit are exempt"
    );
    // ...but an actual systemd.services declaration is LIVE.
    assert_eq!(
        classify(
            "nixos-modules/components/observability/host.nix",
            0,
            &[r#"    systemd.services."nixling-ch-exporter" = {"#.to_string()]
        ),
        Verdict::Live,
        "an actual systemd.services declaration in observability/host.nix must be flagged"
    );

    // Content-based carve-out in an otherwise non-exempt module → skip.
    assert_eq!(
        classify(
            "nixos-modules/network.nix",
            0,
            &[r#"    videoService = "nixling-${m.name}-video";"#.to_string()]
        ),
        Verdict::Skip,
        "a `*Service = \"nixling-...\"` bundle-metadata line is exempt anywhere"
    );

    // Genuine live reference in a non-exempt module → LIVE.
    assert_eq!(
        classify(
            "nixos-modules/network.nix",
            0,
            &[r#"    systemd.services."nixling-net-route-preflight" = {};"#.to_string()]
        ),
        Verdict::Live,
        "a live systemd unit declaration of a retired name must be flagged"
    );
}

/// Guards the denylist itself: every pattern compiles and actually matches a
/// representative retired-unit reference (so a future edit cannot silently
/// neuter a needle into one that never matches).
#[test]
fn legacy_unit_denylist_patterns_match_representative_refs() {
    // One representative retired-unit reference per pattern, in the exact order
    // `denylist_patterns()` returns them.
    let samples: &[&str] = &[
        "systemd.services.\"microvm-tap-interfaces@\"",
        "before microvm-setup@vm.service",
        r#"audioService = "nixling-${m.name}-snd""#,
        "nixling-work-video.service",
        "nixling-work-gpu.service",
        "nixling-work-store-sync.service",
        "nixling-known-hosts-refresh@work.service",
        "nixling-otel-relay@work.service",
        "nixling-net-route-preflight.service",
        "nixling-audit-check.service",
        "nixling-audit-check.timer",
        "g:nixling-ch-exporter",
        "nixling-otel-host-bridge.service",
        "nixling-sys-work-usbipd-3-1.service",
    ];
    let patterns = denylist_patterns();
    assert_eq!(
        patterns.len(),
        samples.len(),
        "denylist pattern count drifted from the representative-sample table"
    );
    for ((raw, re), sample) in patterns.iter().zip(samples.iter()) {
        assert!(
            re.is_match(sample),
            "denylist pattern /{raw}/ failed to match representative reference {sample:?}"
        );
    }

    // The interpolation alternative of NAMEPART must also catch the literal
    // name-segment shape (regression guard for the `${...}` branch).
    let snd = &patterns[2].1;
    assert!(
        snd.is_match("nixling-foo-snd"),
        "NAMEPART literal-segment branch must match a literal unit name"
    );
    assert!(
        snd.is_match(r#"nixling-${cfg.vmName}-snd"#),
        "NAMEPART interpolation branch must match a Nix string-interpolation token"
    );
}
