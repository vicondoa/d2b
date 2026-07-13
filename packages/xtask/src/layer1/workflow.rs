use std::{fs, path::Path};

use super::{
    Layer1Error, Result,
    model::{CiKind, JobSpec, Layer1Manifest},
};

pub const CHECKOUT_ACTION: &str = "actions/checkout@34e114876b0b11c390a56381ad16ebd13914f8d5";
pub const INSTALL_NIX_ACTION: &str =
    "cachix/install-nix-action@23cf0fec1d55e0b1f2631aedd2a610c21ef8b077";
pub const RUST_CACHE_ACTION: &str = "Swatinem/rust-cache@e18b497796c12c097a38f9edb9d0641fb99eee32";
const CLEAR_RUSTC_WRAPPERS: &str = r#"RUSTC_WRAPPER="" CARGO_BUILD_RUSTC_WRAPPER="""#;

pub fn render_workflow(manifest: &Layer1Manifest, template: &str) -> Result<String> {
    manifest.validate()?;
    if template.matches("{{ workflow_name }}").count() != 1
        || template.matches("{{ jobs }}").count() != 1
    {
        return Err(Layer1Error::new(
            "workflow template must contain exactly one {{ workflow_name }} and one {{ jobs }}",
        ));
    }

    let mut rendered_jobs = Vec::new();
    for job_id in &manifest.ci.jobs {
        let job = &manifest.jobs[job_id];
        let rendered = match job.ci_kind.expect("validated ciKind") {
            CiKind::Tier0 => tier0_job(job),
            CiKind::SimpleNix => simple_nix_job(job),
            CiKind::Changelog => changelog_job(job),
            CiKind::Rust => rust_job(job),
            CiKind::FlakeDiscover => flake_discover_job(job),
            CiKind::FlakeX86Shards => flake_x86_shards_job(job),
            CiKind::FlakeX86Outputs => flake_x86_outputs_job(job),
            CiKind::FlakeX86Rollup => flake_x86_rollup_job(job),
            CiKind::FlakeAarch64Smoke => flake_aarch64_smoke_job(job),
        };
        rendered_jobs.push(rendered);
    }
    rendered_jobs.push(check_rollup_job(manifest));

    let workflow = template
        .replace("{{ workflow_name }}", &manifest.ci.workflow_name)
        .replace("{{ jobs }}", &rendered_jobs.join("\n\n"));
    Ok(format!("{}\n", workflow.trim_end()))
}

pub fn check_workflow_file(path: &Path, expected: &str) -> Result<()> {
    let actual = match fs::read_to_string(path) {
        Ok(actual) => actual,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(Layer1Error::new(format!(
                "cannot read generated workflow {}: {error}",
                path.display()
            )));
        }
    };
    check_workflow_text(
        &actual,
        expected,
        &path.display().to_string(),
        &format!("{} (regenerated)", path.display()),
    )
}

pub fn check_workflow_text(
    actual: &str,
    expected: &str,
    actual_label: &str,
    expected_label: &str,
) -> Result<()> {
    if actual == expected {
        return Ok(());
    }
    Err(Layer1Error::new(render_diff(
        actual,
        expected,
        actual_label,
        expected_label,
    )))
}

fn render_diff(actual: &str, expected: &str, actual_label: &str, expected_label: &str) -> String {
    let actual_lines = actual.lines().collect::<Vec<_>>();
    let expected_lines = expected.lines().collect::<Vec<_>>();
    let mut rendered = format!(
        "generated workflow is stale\n--- {actual_label}\n+++ {expected_label}\n@@ -1,{} +1,{} @@\n",
        actual_lines.len(),
        expected_lines.len()
    );
    for line in actual_lines {
        rendered.push('-');
        rendered.push_str(line);
        rendered.push('\n');
    }
    for line in expected_lines {
        rendered.push('+');
        rendered.push_str(line);
        rendered.push('\n');
    }
    rendered
}

fn yaml_list(values: &[String]) -> String {
    format!("[{}]", values.join(", "))
}

fn needs_line(job: &JobSpec) -> String {
    if job.needs.is_empty() {
        String::new()
    } else {
        format!("    needs: {}\n", yaml_list(&job.needs))
    }
}

fn ci_job_id(job: &JobSpec) -> &str {
    job.ci_job_id.as_deref().expect("validated ciJobId")
}

fn runs_on(job: &JobSpec) -> &str {
    job.runs_on.as_deref().expect("validated runsOn")
}

fn timeout(job: &JobSpec) -> u64 {
    job.timeout_minutes.expect("validated timeoutMinutes")
}

fn make_target(job: &JobSpec) -> &str {
    job.make_target.as_deref().expect("validated makeTarget")
}

fn ci_make_command(target: &str) -> String {
    format!("{CLEAR_RUSTC_WRAPPERS} make -- {target}")
}

fn ci_silent_make_command(target: &str) -> String {
    format!("{CLEAR_RUSTC_WRAPPERS} make -s -- {target}")
}

fn nix_setup_step() -> String {
    format!(
        r#"      - uses: {INSTALL_NIX_ACTION}
        with:
          nix_path: nixpkgs=channel:nixos-unstable
          extra_nix_config: |
            experimental-features = nix-command flakes"#
    )
}

fn simple_nix_job(job: &JobSpec) -> String {
    format!(
        r#"  {}:
{}    runs-on: {}
    timeout-minutes: {}
    steps:
      - uses: {}
{}
      - name: {}
        run: {}"#,
        ci_job_id(job),
        needs_line(job),
        runs_on(job),
        timeout(job),
        CHECKOUT_ACTION,
        nix_setup_step(),
        job.display_name,
        ci_make_command(make_target(job))
    )
}

fn tier0_job(job: &JobSpec) -> String {
    format!(
        r#"  {}:
{}    runs-on: {}
    timeout-minutes: {}
    steps:
      - uses: {}
      - name: {}
        run: {}
      - name: ADR index coverage guard
        run: bash tests/unit/meta/adr-index-coverage.sh
      - name: CI coverage structural guard
        run: bash tests/unit/meta/ci-coverage.sh
      - name: Test rearchitecture fail-closed gates
        run: bash tests/tools/gen-migration-ledger.sh --check"#,
        ci_job_id(job),
        needs_line(job),
        runs_on(job),
        timeout(job),
        CHECKOUT_ACTION,
        job.display_name,
        ci_make_command(make_target(job))
    )
}

fn changelog_job(job: &JobSpec) -> String {
    format!(
        r#"  {}:
    if: github.event_name == 'pull_request'
{}    runs-on: {}
    timeout-minutes: {}
    steps:
      - uses: {}
        with:
          fetch-depth: 0
      - name: {}
        run: bash scripts/changelog-check.sh"#,
        ci_job_id(job),
        needs_line(job),
        runs_on(job),
        timeout(job),
        CHECKOUT_ACTION,
        job.display_name
    )
}

fn rust_job(job: &JobSpec) -> String {
    format!(
        r#"  {}:
{}    runs-on: {}
    # Warm (rust-cache hit): ~8-12 min. Cold (no cache): ~43 min.
    timeout-minutes: {}
    env:
      # Disable sccache in CI - we use Swatinem/rust-cache (target-dir
      # caching) instead, which caches ALL compiled artifacts including
      # proc-macros and bin/lib crates that sccache cannot cache (~60% of
      # compilations are "non-cacheable" by sccache due to crate-type).
      # CARGO_INCREMENTAL=0 is still set: incremental compilation artifacts
      # are non-deterministic and bloat the cache without benefit for CI
      # (each PR run starts from a different commit).
      CARGO_INCREMENTAL: "0"
      # Override the repo .cargo/config.toml rustc-wrapper (sccache) so
      # rust-cache's post-step `cargo metadata` doesn't fail looking for
      # an sccache binary that isn't installed.
      RUSTC_WRAPPER: ""
      CARGO_BUILD_RUSTC_WRAPPER: ""
    steps:
      - uses: {}
        with:
          fetch-depth: 0
{}
      - name: Free runner disk for Rust gate
        run: |
          df -h
          sudo rm -rf /usr/local/lib/android /usr/share/dotnet /opt/ghc /usr/local/.ghcup /opt/hostedtoolcache/CodeQL || true
          docker system prune -af || true
          df -h
      - name: Install pinned Rust toolchain + ripgrep + acl
        # MUST run BEFORE Swatinem/rust-cache: the cache action reads
        # `rustc --version` to compute its key hash, so the pinned
        # toolchain must be the active default when the cache step runs.
        # Without this, the runner's pre-installed 1.96.0 is hashed,
        # and the cache is keyed on the wrong compiler version.
        run: |
          PINNED=$(sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' packages/rust-toolchain.toml | head -1)
          rustup toolchain install "$PINNED" --profile minimal --component rustfmt --component clippy
          rustup default "$PINNED"
          echo "Rust toolchain: $(rustc --version)"
          sudo apt-get update && sudo apt-get install -y ripgrep acl
      - name: Rust dependency cache (target dirs + cargo registry)
        # Swatinem/rust-cache caches dependency artifacts in target dirs
        # and the cargo registry. It performs all I/O in its own action
        # process (JavaScript pre/post steps) via @actions/cache - no
        # ACTIONS_RUNTIME_TOKEN or cache credentials are exposed to `run:`
        # steps where untrusted crate code (build scripts, proc-macros,
        # `cargo test`) executes.
        uses: {}
        with:
          workspaces: |
            packages -> target
            packages/d2b-priv-broker -> target
          cache-directories: |
            packages/d2b-priv-broker/target-layer1
            packages/d2b-priv-broker/target-fakebackends
          prefix-key: "v0-rust"
          shared-key: "test-rust-${{{{ runner.os }}}}"
          save-if: "true"
      - name: {}
        # Skip the fixture nix-build (~35 min) - the same fixtures are
        # already evaluated by the flake-eval-x86 (fixture-smoke) and
        # (fixture-smoke-full) shards. The contract tests that depend on
        # D2B_FIXTURES still run in those shards' eval; here we test only
        # the Rust compilation + unit/integration tests.
        env:
          D2B_SKIP_FIXTURE_BUILD: "1"
        run: {}"#,
        ci_job_id(job),
        needs_line(job),
        runs_on(job),
        timeout(job),
        CHECKOUT_ACTION,
        nix_setup_step(),
        RUST_CACHE_ACTION,
        job.display_name,
        ci_make_command(make_target(job))
    )
}

fn flake_discover_job(job: &JobSpec) -> String {
    format!(
        r#"  {}:
{}    runs-on: {}
    timeout-minutes: {}
    outputs:
      checks: ${{{{ steps.list.outputs.checks }}}}
    steps:
      - uses: {}
{}
      - id: list
        name: {}
        run: |
          checks=$({})
          echo "discovered checks: $checks"
          echo "checks=$checks" >> "$GITHUB_OUTPUT"
"#,
        ci_job_id(job),
        needs_line(job),
        runs_on(job),
        timeout(job),
        CHECKOUT_ACTION,
        nix_setup_step(),
        job.display_name,
        ci_silent_make_command("test-flake-list")
    )
}

fn flake_x86_shards_job(job: &JobSpec) -> String {
    let discover_job = &job.needs[0];
    format!(
        r#"  {}:
{}    runs-on: {}
    timeout-minutes: {}
    strategy:
      fail-fast: false
      max-parallel: {}
      matrix:
        check: ${{{{ fromJSON(needs.{}.outputs.checks) }}}}
    steps:
      - uses: {}
      - name: Add swap (insurance for the heaviest single check)
        run: |
          # A single check instantiates in its own process and fits a 16 GB
          # runner (heaviest measured ~12 GB), so unlike the old monolith this
          # rarely touches swap. Add a modest swapfile purely as OOM insurance.
          SWAP=/mnt/d2b-ci-swap
          sudo swapoff "$SWAP" 2>/dev/null || true
          sudo rm -f "$SWAP"
          sudo fallocate -l 8G "$SWAP" || sudo dd if=/dev/zero of="$SWAP" bs=1M count=8192
          sudo chmod 600 "$SWAP"
          sudo mkswap "$SWAP"
          sudo swapon "$SWAP"
{}
      - name: Install flake shard diagnostics
        run: sudo apt-get update && sudo apt-get install -y gdb
      - name: {}
        # D2B_FLAKE_CHECK is passed via the step environment, NOT interpolated
        # into the shell command: a flake check attr name is PR-controlled, so
        # `D2B_FLAKE_CHECK='${{{{ matrix.check }}}}' ...` would be a shell-injection
        # vector. test-flake.sh additionally rejects names outside [A-Za-z0-9._-].
        env:
          D2B_FLAKE_CHECK: ${{{{ matrix.check }}}}
        run: {}"#,
        ci_job_id(job),
        needs_line(job),
        runs_on(job),
        timeout(job),
        job.max_parallel.expect("validated maxParallel"),
        discover_job,
        CHECKOUT_ACTION,
        nix_setup_step(),
        job.display_name,
        ci_make_command("test-flake")
    )
}

fn flake_x86_outputs_job(job: &JobSpec) -> String {
    format!(
        r#"  {}:
{}    runs-on: {}
    timeout-minutes: {}
    steps:
      - uses: {}
{}
      - name: {}
        env:
          D2B_FLAKE_OUTPUTS: "1"
        run: {}"#,
        ci_job_id(job),
        needs_line(job),
        runs_on(job),
        timeout(job),
        CHECKOUT_ACTION,
        nix_setup_step(),
        job.display_name,
        ci_make_command("test-flake")
    )
}

fn flake_x86_rollup_job(job: &JobSpec) -> String {
    let discover_job = &job.needs[0];
    let shards_job = &job.needs[1];
    let outputs_job = &job.needs[2];
    format!(
        r#"  {}:
    needs: {}
    if: always()
    runs-on: {}
    timeout-minutes: {}
    steps:
      - name: {}
        run: |
          discover='${{{{ needs.{}.result }}}}'
          shards='${{{{ needs.{}.result }}}}'
          outputs='${{{{ needs.{}.result }}}}'
          echo "{}=$discover  {}=$shards  {}=$outputs"
          if [ "$discover" = success ] && [ "$shards" = success ] && [ "$outputs" = success ]; then
            echo "All x86_64-linux flake checks + outputs passed."
          else
            echo "::error::x86_64 flake gate failed (discover=$discover, shards=$shards, outputs=$outputs)"
            exit 1
          fi"#,
        ci_job_id(job),
        yaml_list(&job.needs),
        runs_on(job),
        timeout(job),
        job.display_name,
        discover_job,
        shards_job,
        outputs_job,
        discover_job,
        shards_job,
        outputs_job
    )
}

fn flake_aarch64_smoke_job(job: &JobSpec) -> String {
    format!(
        r#"  {}:
{}    runs-on: {}
    timeout-minutes: {}
    steps:
      - uses: {}
{}
      - name: {}
        run: |
          nix-instantiate --eval --strict \
            -E 'let f = import ./tests/unit/smoke/smoke-eval-aarch64.nix; r = f {{}}; in r.drvPath' \
            >/dev/null"#,
        ci_job_id(job),
        needs_line(job),
        runs_on(job),
        timeout(job),
        CHECKOUT_ACTION,
        nix_setup_step(),
        job.display_name
    )
}

fn check_rollup_job(manifest: &Layer1Manifest) -> String {
    let allowed_skipped = manifest
        .ci
        .allowed_skipped_rollup_jobs
        .iter()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    let mut lines = vec![
        format!("  {}:", manifest.ci.rollup_job),
        format!("    needs: {}", yaml_list(&manifest.ci.rollup_needs)),
        "    if: always()".to_owned(),
        "    runs-on: ubuntu-latest".to_owned(),
        "    timeout-minutes: 5".to_owned(),
        "    steps:".to_owned(),
        "      - name: Require generated Layer-1 gate graph to pass".to_owned(),
        "        run: |".to_owned(),
        "          failed=0".to_owned(),
        "          require_success() {".to_owned(),
        "            name=\"$1\"".to_owned(),
        "            result=\"$2\"".to_owned(),
        "            echo \"$name=$result\"".to_owned(),
        "            if [ \"$result\" != success ]; then".to_owned(),
        "              echo \"::error::$name did not pass (result=$result)\"".to_owned(),
        "              failed=1".to_owned(),
        "            fi".to_owned(),
        "          }".to_owned(),
        "          allow_success_or_skipped() {".to_owned(),
        "            name=\"$1\"".to_owned(),
        "            result=\"$2\"".to_owned(),
        "            echo \"$name=$result\"".to_owned(),
        "            if [ \"$result\" != success ] && [ \"$result\" != skipped ]; then".to_owned(),
        "              echo \"::error::$name did not pass (result=$result)\"".to_owned(),
        "              failed=1".to_owned(),
        "            fi".to_owned(),
        "          }".to_owned(),
    ];
    for need in &manifest.ci.rollup_needs {
        let expression = format!("${{{{ needs.{need}.result }}}}");
        let function = if allowed_skipped.contains(need.as_str()) {
            "allow_success_or_skipped"
        } else {
            "require_success"
        };
        lines.push(format!("          {function} {need} '{expression}'"));
    }
    lines.extend([
        "          if [ \"$failed\" -ne 0 ]; then".to_owned(),
        "            exit 1".to_owned(),
        "          fi".to_owned(),
        "          echo \"All generated Layer-1 jobs passed.\"".to_owned(),
    ]);
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn committed_manifest() -> Layer1Manifest {
        serde_json::from_str(include_str!("../../../../tests/layer1-jobs.json"))
            .expect("committed manifest")
    }

    #[test]
    fn rendering_is_deterministic_and_matches_committed_workflow() {
        let manifest = committed_manifest();
        let template = include_str!("../../../../tests/ci/layer1-workflow.template.yml");
        let first = render_workflow(&manifest, template).expect("first render");
        let second = render_workflow(&manifest, template).expect("second render");
        assert_eq!(first, second);
        assert_eq!(
            first,
            include_str!("../../../../.github/workflows/pr-l1-static-fast.yml")
        );
        assert!(first.contains(CHECKOUT_ACTION));
        assert!(first.contains(INSTALL_NIX_ACTION));
        assert!(first.contains(RUST_CACHE_ACTION));
        assert!(first.contains("  pull_request:\n  push:\n    branches: [main]"));
        assert!(!first.contains("  pull_request:\n    branches: [main]"));
        assert!(
            first.contains(
                r#"run: RUSTC_WRAPPER="" CARGO_BUILD_RUSTC_WRAPPER="" make -- test-lint"#
            )
        );
        assert!(first.contains(
            r#"checks=$(RUSTC_WRAPPER="" CARGO_BUILD_RUSTC_WRAPPER="" make -s -- test-flake-list)"#
        ));
        assert!(first.contains("      RUSTC_WRAPPER: \"\"\n      CARGO_BUILD_RUSTC_WRAPPER: \"\""));
        assert!(!first.contains("run: make "));
    }

    #[test]
    fn generated_rollup_distinguishes_required_and_skippable_roots() {
        let manifest = committed_manifest();
        let template = include_str!("../../../../tests/ci/layer1-workflow.template.yml");
        let rendered = render_workflow(&manifest, template).expect("render");
        assert!(rendered.contains("require_success tier0 '${{ needs.tier0.result }}'"));
        assert!(rendered.contains(
            "allow_success_or_skipped test-changelog '${{ needs.test-changelog.result }}'"
        ));
        assert!(!rendered.contains("allow_success_or_skipped tier0 "));

        let mut invalid = manifest;
        invalid
            .ci
            .rollup_needs
            .retain(|job_id| job_id != "test-policy");
        invalid
            .jobs
            .get_mut("test-changelog")
            .expect("test-changelog")
            .needs
            .push("test-policy".to_owned());
        let error = render_workflow(&invalid, template)
            .expect_err("skippable root must not provide the only coverage path");
        assert!(error.to_string().contains("non-skippable rollup root"));
    }

    #[test]
    fn tier0_jobs_render_validated_needs_and_preserve_transitive_rollup_coverage() {
        let mut manifest = committed_manifest();
        let mut bootstrap = manifest.jobs["tier0"].clone();
        bootstrap.display_name = "Bootstrap Tier 0".to_owned();
        bootstrap.ci_job_id = Some("bootstrap".to_owned());
        bootstrap.needs.clear();
        manifest.jobs.insert("bootstrap".to_owned(), bootstrap);
        manifest.local.phases[0]
            .jobs
            .insert(0, "bootstrap".to_owned());
        manifest.jobs.get_mut("tier0").expect("tier0").needs = vec!["bootstrap".to_owned()];
        manifest.ci.jobs.insert(0, "bootstrap".to_owned());

        manifest
            .validate()
            .expect("tier0 dependency is covered transitively by the tier0 rollup root");
        assert!(!manifest.ci.rollup_needs.contains(&"bootstrap".to_owned()));
        let rendered = render_workflow(
            &manifest,
            include_str!("../../../../tests/ci/layer1-workflow.template.yml"),
        )
        .expect("render");
        assert!(rendered.contains("  tier0:\n    needs: [bootstrap]\n    runs-on:"));
    }

    #[test]
    fn check_mode_fails_on_drift_and_accepts_exact_bytes() {
        let expected = "name: generated\n";
        check_workflow_text(expected, expected, "actual", "expected").expect("no drift");
        let error = check_workflow_text(
            "name: edited\n",
            expected,
            "workflow.yml",
            "workflow.yml (regenerated)",
        )
        .expect_err("drift");
        let message = error.to_string();
        assert!(message.contains("generated workflow is stale"));
        assert!(message.contains("-name: edited"));
        assert!(message.contains("+name: generated"));
    }
}
