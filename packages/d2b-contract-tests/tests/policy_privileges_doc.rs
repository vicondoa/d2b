//! Legacy-systemd-surface obituary completeness lint (the "H-group"),
//! migrated from `tests/privileges-doc-completeness-eval.sh`. The gate is a
//! pure doc-vs-`nixos-modules/` string-manipulation completeness scanner: for
//! every legacy systemd template / singleton the framework historically
//! emitted, `docs/reference/privileges.md` must carry either a live broker-op
//! row OR a documented retirement (obituary) — never a self-contradictory both.
//! It runs grep-only (no Nix eval/build), so it ports cleanly to Rust string
//! parsing.
//!
//! This crate is excluded from the hermetic Nix sandbox workspace build and
//! runs only from `tests/tools/rust-workspace-checks.sh` against the real checkout,
//! so reading repo files via the `d2b_contract_tests` repo-file helpers is
//! sound here.
//!
//! Migrated gate:
//!   * tests/privileges-doc-completeness-eval.sh -> legacy_systemd_surface_obituary_completeness
//!
//! Self-referential-doc note: `docs/reference/privileges.md` (the doc this
//! test scans) names the retired `tests/privileges-doc-completeness-eval.sh`
//! script in prose. That filename is deliberately NOT used as a content marker
//! here, and nothing in this test asserts the script's continued existence, so
//! retiring the `.sh` keeps this test green. The doc prose reference is left
//! for the integrator's sweep.

use std::fs;
use std::path::{Path, PathBuf};

use d2b_contract_tests::{read_repo_file, repo_path_exists, repo_root};
use regex::Regex;

const DOC_REL: &str = "docs/reference/privileges.md";
const MODULES_REL: &str = "nixos-modules";

/// The canonical obituary-section heading. The bash gate locates it with
/// `grep -n '^## Legacy systemd surface obituary'`, i.e. a starts-with match.
const OBIT_HEADING: &str = "## Legacy systemd surface obituary";

/// Canonical legacy patterns (`LEGACY_UNITS` in the bash gate). Each entry is a
/// regex anchored on the unit base name; the same pattern matches
/// `systemd.services.<x>` Nix attrs and `<x>.service`/`<x>@<vm>.service` doc
/// citations.
const LEGACY_UNITS: &[&str] = &[
    // Per-VM templates.
    "d2b@",
    "microvm@",
    "microvm-tap-interfaces@",
    "microvm-set-booted@",
    "microvm-pci-devices@",
    "microvm-virtiofsd@",
    r#"d2b-[^"@ ]+-gpu"#,
    r#"d2b-[^"@ ]+-video"#,
    r#"d2b-[^"@ ]+-snd"#,
    r#"d2b-[^"@ ]+-swtpm"#,
    r#"d2b-[^"@ ]+-store-sync"#,
    "d2b-known-hosts-refresh@",
    "d2b-vfsd-watchdog@",
    "d2b-otel-relay@",
    // Host singletons.
    "d2b-net-route-preflight",
    "d2b-audit-check",
    "d2b-ch-exporter",
    "d2b-otel-host-bridge",
    r#"d2b-sys-[^"@ ]+-usbipd-proxy"#,
    r#"d2b-sys-[^"@ ]+-usbipd-backend"#,
];

/// A line in the live region carries an obituary marker if it mentions any of
/// these phrases — they signal "this row is the obituary in-place, not a
/// contradictory live row". (`LIVE_OBIT_MARKERS` in the bash gate.)
const LIVE_OBIT_MARKERS: &str = "Retired|retired|retires|deleted|obituary|MUST NOT|\
scheduled.for.removal|folding their work|re-homed|replaced by|replacement|\
current surface|no longer exists|not emitted";

/// Recursively collect every regular file under `dir`. Mirrors `grep -r`, which
/// descends the whole tree and inspects every file (this gate's
/// `nixos-modules/` tree is all text — `.nix`/`.rs`/`.py`/`.sh`/`.json`/
/// `.toml`).
fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("policy-lint: cannot read dir {}: {err}", dir.display()));
    for entry in entries {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out);
        } else if path.is_file() {
            out.push(path);
        }
    }
}

/// All lines of all files under `nixos-modules/`, read lossily so non-UTF-8
/// bytes can't abort the scan (faithful to `grep`, which still line-scans such
/// files). A legacy pattern is "emitted" if any single line matches; keeping
/// lines per-file-flattened into one vector preserves grep's per-line, any-file
/// semantics without introducing cross-file boundary matches.
fn module_lines() -> Vec<String> {
    let mut files = Vec::new();
    collect_files(&repo_root().join(MODULES_REL), &mut files);
    files.sort();
    let mut lines = Vec::new();
    for path in files {
        let bytes = fs::read(&path)
            .unwrap_or_else(|err| panic!("policy-lint: cannot read {}: {err}", path.display()));
        for line in String::from_utf8_lossy(&bytes).lines() {
            lines.push(line.to_string());
        }
    }
    lines
}

// ---------------------------------------------------------------------------
// Migrated from tests/privileges-doc-completeness-eval.sh.
//
// For each legacy unit pattern the gate inspects two doc regions of
// docs/reference/privileges.md:
//
//   * the obituary region = lines from the
//     `## Legacy systemd surface obituary` heading through (and including) the
//     next top-level `## ` heading — the canonical index of retired units;
//   * the live region      = every other line — the broker-op / runner-role /
//     DAG-node surface that is the daemon-only end-state.
//
// Hard-fail modes:
//   (1) emitted by nixos-modules/ but mentioned nowhere in the doc;
//   (2) no longer emitted AND absent from the obituary index;
//   (3) a live (unmarked) doc row AND an obituary row — contradictory.
//
// Transitional state (still emitted AND already in the obituary) is a benign
// warning, not a failure — the doc lands before the code-deletion sibling.
// ---------------------------------------------------------------------------
#[test]
fn legacy_systemd_surface_obituary_completeness() {
    assert!(
        repo_path_exists(DOC_REL),
        "privileges-doc-completeness: privileges doc not found: {DOC_REL}"
    );
    assert!(
        repo_root().join(MODULES_REL).is_dir(),
        "privileges-doc-completeness: nixos-modules dir not found: {MODULES_REL}"
    );

    let doc = read_repo_file(DOC_REL);
    let doc_lines: Vec<&str> = doc.lines().collect();
    let doc_total = doc_lines.len();
    assert!(
        doc_total > 0,
        "privileges-doc-completeness: doc read produced no lines: {DOC_REL}"
    );

    // OBIT_START: 1-based line number of the obituary heading (starts-with,
    // first match — mirrors `grep -n '^## ...' | head -1`).
    let obit_start = doc_lines
        .iter()
        .position(|l| l.starts_with(OBIT_HEADING))
        .map(|i| i + 1)
        .unwrap_or_else(|| {
            panic!("privileges-doc-completeness: doc missing '{OBIT_HEADING}' section")
        });

    // OBIT_END: 1-based line number of the first top-level `## ` heading AFTER
    // the obituary heading; falls back to the doc length when none follows
    // (mirrors the awk scan + `wc -l` fallback). Sub-headings (`### `) do not
    // terminate the region.
    let obit_end = doc_lines
        .iter()
        .enumerate()
        .skip(obit_start) // 0-based index obit_start == 1-based line obit_start+1 (i.e. NR>s)
        .find(|(_, l)| l.starts_with("## "))
        .map(|(i, _)| i + 1)
        .unwrap_or(doc_total);

    // Obituary region: 1-based lines [obit_start, obit_end] inclusive.
    let obit_lines: &[&str] = &doc_lines[(obit_start - 1)..obit_end];
    // Live region: every line NOT in [obit_start, obit_end].
    let live_lines: Vec<&str> = doc_lines
        .iter()
        .enumerate()
        .filter(|(i, _)| {
            let ln = i + 1;
            !(ln >= obit_start && ln <= obit_end)
        })
        .map(|(_, l)| *l)
        .collect();
    let live_n = live_lines.len();

    let module_lines = module_lines();
    let marker_re = Regex::new(LIVE_OBIT_MARKERS).expect("valid LIVE_OBIT_MARKERS regex");

    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    for pat in LEGACY_UNITS {
        // emitted: `grep -rEq "systemd\.services\."?<pat>"` over nixos-modules/.
        let emitted_re = Regex::new(&format!(r#"systemd\.services\."?{pat}"#))
            .unwrap_or_else(|err| panic!("invalid emitted regex for '{pat}': {err}"));
        let emitted = module_lines.iter().any(|l| emitted_re.is_match(l));

        // Doc citations must look like an actual systemd unit name — require the
        // pattern to abut a `.service`/`.socket`/`.timer`/`{`, an `@`, or a
        // `<vm>.` reference. Bare uid/principal mentions are not unit-name
        // citations.
        let doc_pat = format!(r#"{pat}(\.(service|socket|timer|\{{)|@|<vm>\.)"#);
        let doc_re = Regex::new(&doc_pat)
            .unwrap_or_else(|err| panic!("invalid doc_pat regex for '{pat}': {err}"));

        let in_obit = obit_lines.iter().any(|l| doc_re.is_match(l));

        // Count live-region mentions whose surrounding ±3 lines lack any
        // obituary marker (a bare/contradictory live row).
        let mut in_live_any = false;
        let mut bare_live_hits = 0usize;
        for (li, line) in live_lines.iter().enumerate() {
            if !doc_re.is_match(line) {
                continue;
            }
            in_live_any = true;
            let lo = li.saturating_sub(3);
            let hi = (li + 3).min(live_n - 1);
            let marked = (lo..=hi).any(|wi| marker_re.is_match(live_lines[wi]));
            if !marked {
                bare_live_hits += 1;
            }
        }

        // Failure (1): emitted by nixos-modules/ but undocumented.
        if emitted && !in_live_any && !in_obit {
            errors.push(format!(
                "'{pat}' is emitted by nixos-modules/ but mentioned nowhere in {DOC_REL}"
            ));
            continue;
        }

        // Failure (2): deleted but undocumented.
        if !emitted && !in_obit {
            errors.push(format!(
                "'{pat}' is no longer emitted but has no obituary row"
            ));
            continue;
        }

        // Failure (3): self-contradictory live row + obituary.
        if bare_live_hits > 0 && in_obit {
            errors.push(format!(
                "'{pat}' has a live (unmarked) doc row AND an obituary row — contradictory"
            ));
            continue;
        }

        // Transitional warning: still emitted AND already in the obituary.
        if emitted && in_obit {
            warnings.push(format!(
                "'{pat}' still emitted by nixos-modules/ but already in obituary \
                 (transitional; systemd emission removal pending)"
            ));
        }
    }

    assert!(
        errors.is_empty(),
        "privileges-doc completeness violation(s):\n  {}\n(transitional warnings: {})",
        errors.join("\n  "),
        warnings.len()
    );
}
