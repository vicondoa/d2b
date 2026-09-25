# The deny.toml policy file

`SKILL.md` carries the judgment calls; this file carries the policy in full,
annotated. Paste the starter as-is, then adapt section by section — every
change to the file is a decision, and a decision worth writing down in the
same place the tool reads it.

## A starting deny.toml

Every section present from day one, so an empty section is a visible choice
rather than an unnoticed default:

```toml
[advisories]
db-path = "~/.cargo/advisory-db"
db-urls = ["https://github.com/rustsec/advisory-db"]
yanked = "deny"
ignore = []

[licenses]
allow = ["MIT", "Apache-2.0", "BSD-3-Clause", "ISC", "Unicode-3.0"]
confidence-threshold = 0.9
exceptions = []

[bans]
multiple-versions = "warn"
wildcards = "deny"
deny = []
skip = []

[sources]
unknown-registry = "deny"
unknown-git = "deny"
allow-git = []
```

Key by key:

- `[advisories] db-path` — where the local clone of the advisory database
  lives. The tool refreshes it before each check; the tree is judged against
  what the database says on the day of the run, not on the day the policy was
  written.
- `[advisories] db-urls` — the origin of that clone, the RustSec database.
  This is the only URL in the policy, and it is functional configuration, not
  a citation.
- `[advisories] yanked` — treat versions yanked from the registry as
  findings. `deny` keeps the tree from depending on a version that a fresh
  consumer could no longer build.
- `[advisories] ignore` — per-advisory exceptions. Both forms are below; every
  entry carries a reason and a review date.
- `[licenses] allow` — the explicit allow-list. Nothing in the tree is
  accepted unless it is named here; the default stance is deny-by-omission.
- `[licenses] confidence-threshold` — when the tool cannot identify a licence
  with at least this much confidence, it reports a finding instead of
  guessing. Keep it at or above 0.9.
- `[licenses] exceptions` — per-crate overrides for crates whose metadata is
  wrong or missing entirely. The form is below.
- `[bans] multiple-versions` — the level at which two resolved versions of one
  crate report. The starter says `warn` on purpose; see the note after the
  list.
- `[bans] wildcards` — `*` version requirements are a range nobody chose, so
  the starter denies them.
- `[bans] deny` — crates banned outright, by name.
- `[bans] skip` — crates exempted from the ban list. Use sparingly, with a
  comment, for the same reason `ignore` entries carry reasons.
- `[sources] unknown-registry` — deny anything that does not come from
  crates.io.
- `[sources] unknown-git` — deny anything that comes from a git source not
  named in `allow-git`.
- `[sources] allow-git` — the git sources the repo has deliberately adopted,
  pinned with a reason in the commit that adds them.

The one warning the starter bakes in: `multiple-versions = "deny"` on an
existing repo produces dozens of failures on day one, because the tree
already carries duplicates the previous owner accepted. Start at `warn`, drive
the count down, then flip it — a `deny` that cannot pass is a `deny` that gets
deleted.

## How to accept a finding without lying about it

Two forms, in the order of preference. The annotated form first:

```toml
[advisories]
ignore = [
  # RUSTSEC-0000-0000: reached only from a build-time dev tool, not shipped.
  # Re-check when <crate> 2.0 lands. Reviewed 2026-08-15.
  { id = "RUSTSEC-0000-0000", reason = "build-time only; not in the shipped binary" },
]
```

The bare form is a plain ID in the same list — acceptable for one release
cycle while the reason is being worked out, never as the resting state:

```toml
[advisories]
ignore = ["RUSTSEC-0000-0001"]
```

Every entry carries who decided, why, and what would change the decision. The
comment is where the reasoning lives — reachability, the re-check trigger,
the review date — and the `reason` field is where the tool can read it. An
`ignore` list without reasons becomes permanent within two releases: nobody
remembers why it is there, so nobody ever removes it, and the next reviewer
treats the list as the baseline rather than the exception.

## A licence exception

For a crate with no licence metadata at all — or metadata that is wrong — the
per-crate form, with the reason in the comment because the field has nowhere
else to go:

```toml
[[licenses.exceptions]]
# example-crate ships no licence file and no licence key in the manifest.
# The upstream README states MIT; verify before the next dependency refresh.
crate = "example-crate"
license = "MIT"
```

The `[[licenses.exceptions]]` block replaces the `exceptions = []` line in the starter file — TOML will not parse both in one file.

Note the interaction with `confidence-threshold`: below about 0.8, the tool
starts matching licences it is guessing at, and a low threshold turns
"unclear licence" into "accepted licence." Raise the threshold and let the
finding surface; solve it with an exception that names the reason, not with a
threshold that hides it.

## The [patch.crates-io] escape hatch

For the case where an advisory has a fix upstream that is not released yet:
point the build at the fixed commit instead of waiting for the release. The
block belongs in the workspace-root `Cargo.toml`, not in `deny.toml` — the
build reads it, the policy tool does not.

```toml
[patch.crates-io]
# RUSTSEC-0000-0002: the fix is merged upstream, not yet published.
# Delete this entry when the fixed version lands on crates.io.
example-crate = { git = "<upstream-repo>", rev = "<fix-commit>" }
```

The caveat that makes this a decision and not a reflex: a `[patch]` entry is
invisible to consumers of a published library. If this repo is a library, the
patch fixes only the build — every downstream crate still resolves the
vulnerable version from the registry, and the advisory remains on their tree.
For a binary the patch is complete; for a library it is a stopgap that the
next report must say so of.

## Three worked scenarios

**A binary crate with a permissive-only policy.** The starter file stands
almost as-is: the `allow` list is already permissive families, so the first
run settles the licences question. An advisory on a network-facing path is an
upgrade — direct or through the intermediary, per `cargo tree --invert`. An
advisory that only build-time tooling reaches earns an annotated `ignore`
entry with a re-check date, or a `[patch]` entry if the fix is merged but
unreleased. Nothing is published, so the patch-invisibility caveat does not
apply; the patch is a complete fix until the release lands.

**A library that must stay MSRV-compatible, and the fix needs a newer
compiler.** The interesting case, and the one where the report must not pretend
there is a free option. The advisory is real, the fix exists, and taking it
bumps a dependency past the MSRV recorded in `docs/agents/rust.md`. The
decision is between the MSRV commitment and the advisory, and it belongs to
the repo, with the trade stated on the record:

- Raise the MSRV. For a published library this is a semver decision with
  downstream cost; it is the user's call, made with the cost named.
- Patch via `[patch.crates-io]`. Fixes the local build only — invisible to
  every consumer, so the report says the advisory remains on downstream trees.
- Vendor the fixed version. Complete, but the maintenance burden follows the
  repo for as long as the vendored copy lives.
- Accept with an expiry. The advisory stays on the tree with a written reason,
  a reviewer, and a date that forces the question again.

Whichever is picked, the report records the choice and the reason the other
three were not. A library that silently raises the MSRV to dodge an advisory
has traded a vulnerability for a breakage with worse optics.

**A workspace with an unavoidable duplicate.** `cargo tree -d` shows two
versions of one crate, pulled by two frameworks whose ranges genuinely do not
unify — no lower-bound raise reaches both. Banning one framework is a product
decision, not a supply-chain one, so the policy decision is narrower: keep
`multiple-versions = "warn"`, name the duplicate and both pullers in a comment
at the `[bans]` section, and set the re-check trigger — whichever framework
next releases a version with a compatible range. The duplicate stays visible
on every run until it is resolved; the comment is what makes a future
`warn`-to-`deny` flip that fails on this known, accepted fact an expected cost,
not a mystery.
