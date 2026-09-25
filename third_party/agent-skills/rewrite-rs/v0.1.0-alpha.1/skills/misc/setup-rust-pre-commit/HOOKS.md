# Pre-commit hooks, in full

The three mechanisms ready to paste, the rewrite variant, and the contributing-notes
block. `SKILL.md` carries the judgment — which mechanism, what goes in the hook, how
the level is derived — and this file carries the files themselves.

## The `core.hooksPath` script

At `.githooks/pre-commit`:

```bash
#!/usr/bin/env bash
set -euo pipefail

staged=$(git diff --cached --name-only --diff-filter=ACM -- '*.rs')
[ -z "$staged" ] && exit 0

if ! cargo fmt --check -- $staged; then
  echo "pre-commit: run 'cargo fmt' and re-stage." >&2
  exit 1
fi

cargo clippy --all-targets -- --no-deps
```

Make it executable and point the clone at it:

```bash
chmod +x .githooks/pre-commit
git config core.hooksPath .githooks
```

`core.hooksPath` is per-clone: nothing in the repo enforces that a contributor
ran the `git config` line, so the install belongs in the contributing notes, and
CI must never depend on it having been run.

## `lefthook.yml`

```yaml
pre-commit:
  parallel: true
  commands:
    fmt:
      glob: "*.rs"
      run: cargo fmt --check -- {staged_files}
    clippy:
      glob: "*.rs"
      run: cargo clippy --all-targets
```

`parallel: true` runs the two commands at once, which is what keeps the combined
hook inside the two-second budget. The one cost is the binary itself — install
it once per machine.

## `.pre-commit-config.yaml`

Local hooks rather than a third-party mirror, so the cargo version in play is the
one the repo controls:

```yaml
repos:
  - repo: local
    hooks:
      - id: cargo-fmt
        name: cargo fmt
        entry: cargo fmt --check --
        language: system
        types: [rust]
      - id: cargo-clippy
        name: cargo clippy
        entry: cargo clippy --all-targets
        language: system
        types: [rust]
        pass_filenames: false
```

`pass_filenames: false` on clippy, explained: clippy takes crates, not files.
Passing filenames to it produces an invocation that silently lints the wrong
thing — the command runs over the workspace either way, and the hook looks like
it checked what it did not check. The `fmt` hook keeps the default, because
`cargo fmt --check --` does take a file list.

## The rewrite-and-restage variant

The variants above report: `cargo fmt --check` fails, and the user formats. This
one rewrites instead — and only with the restage and the printout, because a
rewrite that goes unprinted changes what the user is about to commit without
telling the user.

For the `core.hooksPath` script, the full variant:

```bash
#!/usr/bin/env bash
set -euo pipefail

staged=$(git diff --cached --name-only --diff-filter=ACM -- '*.rs')
[ -z "$staged" ] && exit 0

dirty=$(git diff --name-only -- $staged)
if [ -n "$dirty" ]; then
  echo "pre-commit: staged files also carry unstaged edits:" >&2
  echo "$dirty" >&2
  echo "pre-commit: stage or stash them, then commit again." >&2
  exit 1
fi

cargo fmt -- $staged
touched=$(git diff --name-only -- $staged)
if [ -n "$touched" ]; then
  printf 'pre-commit: reformatted and re-staged:\n' >&2
  printf '%s\n' "$touched" >&2
  git add -- $touched
fi

cargo clippy --all-targets -- --no-deps
```

The `dirty` guard matters: `cargo fmt` rewrites the working tree, so a staged
file that also carries unstaged edits would get formatted at the unstaged state
and re-staged whole, sweeping the unstaged work into the commit. The guard stops
the hook and names the files instead.

For `lefthook.yml`, drop `--check` from the `fmt` run line:
`cargo fmt -- {staged_files}`. For `.pre-commit-config.yaml`, drop it from the
entry: `cargo fmt --`. Both then need the same guard, restage, and printout the
script above carries — refuse files with unstaged edits, re-stage exactly the
files the formatter touched, and print every one — folded into the `fmt` command
if the mechanism does not do it on its own.

## The contributing-notes block

Paste into the contributing notes of the target repo — how to install, what
runs, and how to bypass. Written for the `core.hooksPath` script; swap the
install line for the mechanism the repo picked (`pre-commit install` for
pre-commit, the lefthook binary for lefthook):

```markdown
### Pre-commit hook

Install, once per clone:

    chmod +x .githooks/pre-commit
    git config core.hooksPath .githooks

What it runs, in under two seconds once the target directory is warm:
`cargo fmt --check` on the staged Rust files, then
`cargo clippy --all-targets -- --no-deps` at the lint level the repo configured.
Format and lint only — tests, builds, and anything network-bound run in CI,
and CI is the gate.

Bypass: `git commit --no-verify`. The hook is a convenience, not a gate. Use
the bypass for a work-in-progress commit or an emergency fix; CI reports what
the hook skipped.
```
