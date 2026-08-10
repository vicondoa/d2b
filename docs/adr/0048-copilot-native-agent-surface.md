# ADR 0048: Copilot-native agent, skill, and panel surface

- Status: Accepted
- Date: 2026-08-01
- Partially superseded by: [ADR 0055](0055-discover-fix-verify-panel-review.md)
  for panel selection, seat identity, model and effort policy, lifecycle, and
  versioned record shape. The Copilot-only definition surface, measured
  dispatch mechanism, read-only reviewer construction, helper assembly
  principle, standalone ADR process, and qualified wave identifiers remain.
- Related: ADR 0046 (d2b 3.0 provider control plane), and the delivery
  tooling in `packages/xtask/src/delivery/` that ADR 0046 section 12.3 uses

## Context

This repository runs a heavyweight engineering process: ADR authoring,
spec-kit feature specs, selected-roster panel review, and an attest/seal/
merge-eligibility gate implemented in `packages/xtask/src/delivery/`.

### Historical state (retired)

Before the immediate cutover recorded by this ADR, the process also had an
OpenCode integration: spec-kit selected `integration: opencode`, the panel
existed as a single generic subagent in `.opencode/agents/panel.md`, and
`AGENTS.md` named `.opencode/opencode.json` as the reference implementation
for panel behaviour. This paragraph records the former state only; those
files and that integration are no longer installed, selected, or supported.

The operator drives Copilot, frequently over ACP, where interactive commands
such as `/model` are not reliably available. Model selection therefore has to
be declared in committed files rather than chosen at a prompt.

Two properties of the gate at adoption made that requirement sharp rather than
cosmetic. `packages/xtask/src/delivery/panel.rs` attested each record's
`provider`, `model_version`, and `reasoning_effort`, then pinned to
`github-copilot` / `gemini-3.1-pro-preview` / `high`. The panel model was
deliberately not the coding model, so a lane could not both author a change and
attest to it. A record that claims a binding the lane did not actually run at
is therefore not a cosmetic error; it is a false attestation on the gate that
seals a wave. ADR 0055 supersedes the particular roster, model, effort, and
record shape; its versioned selection and current dispatch table are
authoritative.

The cutover is immediate. Existing delivery records remain historical
evidence, not an executable integration or an authority over the committed
Copilot surface.

## Decision

Make Copilot the sole authoritative definition surface. Agents under
`.github/agents/` and d2b skills under `.github/skills/` define the process.
The `.opencode/` integration surface is removed at this cutover and must not be
recreated as a compatibility path.

### The binding mechanism

Everything below was **measured against the installed Copilot CLI 1.0.75** by
creating real files and observing actual behaviour. Several results contradict
published guidance, so this section records the measurement rather than the
documentation, and it should be re-measured on every CLI upgrade.

What works:

- `model:` in `.github/agents/<name>.agent.md` frontmatter is honoured.
- `tools:` in that frontmatter is mechanically enforced. An agent declaring
  `tools: [view, grep, glob]` has no shell at all.
- Task-tool `model`, `reasoning_effort`, and `context_tier` at dispatch are
  causal, and vary **per lane inside a single session**. One session was
  observed producing `gemini-3.1-pro-preview:high`, `gpt-5.6-sol:xhigh`, and
  `gemini-3.1-pro-preview:low` while the parent stayed on `claude-opus-5:xhigh`.

What does not work:

- `effortLevel:` and `contextTier:` in agent frontmatter are warned and ignored.
- `reasoningEffort:` in agent frontmatter is accepted **with no warning** and is
  inert. This is the most dangerous shape available, because it looks applied.
- Repo-scope `.github/copilot/settings.json` honours neither `model` nor
  `subagents`. Its keys pass through a fixed allowlist compiled into the CLI:
  `model`, `remoteControl`, `enabledPlugins`, `extraKnownMarketplaces`,
  `permissions`, `shellShortcut`, `strictKnownMarketplaces`, `telemetry`,
  `allowedMcpServers`, `deniedMcpServers`. `subagents` is absent, so the block
  that works at user scope is silently dropped at repo scope. There is no
  `--settings` or `--config` flag.
- `--agent <name>` is ignored over ACP. It binds a session in print mode only,
  so a launcher-per-role design does not work on the transport in use.
- A subagent does **not** inherit the session's reasoning effort. With a parent
  at `claude-opus-5:xhigh`, an unpinned Gemini lane ran at `medium`, that
  model's own default.
- An agent with no frontmatter `model`, dispatched with no parameters, inherits
  the **parent session's** model.

Therefore: **dispatch parameters carry the binding**, and the binding table
lives in the committed skill markdown. Frontmatter `model:` is kept as well,
even though the tables always pass `model` explicitly, because the two failure
modes differ. An agent that omits `model` and is hand-invoked runs on the
caller's model; an agent that pins it runs the right model and loses only the
effort. The record helper can reject a declared effort that disagrees with
policy, but it cannot prove the effort actually used. One line per agent makes
a false model declaration require two independent mistakes.

Nothing modifies the operator's `~/.copilot/settings.json`. Per-lane dispatch
was measured sufficient with no `subagents` block in either scope.

### Configuration checks do not prove execution

An unpinned panel lane runs at the model default while its record would attest
the policy effort. The committed checks reduce configuration drift, but the
same-user process metadata is not an authenticated execution receipt and
cannot detect a lying declaration:

1. the committed dispatch tables, which make it rarely happen;
2. `scripts/copilot/check-bindings.mjs`, which rejects a missing row, an
   illegal effort for a model, a disagreement with the delivery policy
   constants, or any effort-like frontmatter key, before a run starts;
3. the record helper, which requires declared process metadata and rejects a
   declaration that disagrees with the completion-bound policy rather than
   defaulting missing values.

The residual risk is explicit: without authenticated execution receipts, these
checks validate configuration and declarations, not the effort actually used.

### Panel agents are read-only by construction

Copilot agent frontmatter has no per-command permission allowlist, so a legacy
shell rule such as "allow `git diff*`, deny the rest" does not translate. The
panel agents instead declare `tools: [view, grep, glob]`, and the panel skill
pre-stages `delta.diff` and `full.diff` for them to read. This is stronger than
granting a restricted shell: it is mechanical rather than prompt-enforced, it
keeps selected lanes off the shared Nix store and the heavy-gate semaphore, and
every reviewer in a lifecycle sees staged evidence.

### Verdicts are authored by agents; records are assembled by a helper

At adoption, `PanelRecord` was a fourteen-field `deny_unknown_fields` struct
requiring `candidate_id`, `content_id`, `snapshot_sha256`, `output_sha256`,
`run_id`, and `receipt_locator`. ADR 0055 supersedes that fixed shape with the
current versioned selected-roster records. The retained decision is that a
reviewing agent emits only its verdict and a bundled helper joins it to the
candidate-bound metadata the agent cannot know. Prompts stay small and digest
handling stays in one testable place.

### Independent agents, not one

Each role gets its own agent with its own domain checklist anchored to this
repository's invariants. The cost is deliberate. An early panel here returned
zero sign-offs with eleven high findings that the static gate caught none of,
and the five-seat council is already documented as a synthesis risk where five
synthesizers agree in places independent reviewers would have dissented.

### spec-kit authors; the d2b skills execute

spec-kit ships a workflow runner with a rich step vocabulary and per-step
`model`, which is tempting for the panel. It is the wrong tool here for one
concrete reason: every step is dispatched as a subprocess and there is **no
per-step reasoning effort**; the only knob is process-global. A panel whose
records attest a pinned `reasoning_effort` cannot be driven by a runner that
cannot set effort per lane. So spec-kit authors the artifacts through
in-session slash commands, and the d2b skills execute the work through
in-session Task lanes where all three parameters bind per lane.

spec-kit is installed in **skills** mode rather than the default markdown mode,
which keeps roughly twenty spec-kit agents out of `.github/agents/` and avoids
the `--agent` dispatch that ACP ignores.

### The ADR process stays separate

An ADR is its own run with its own PR and its own merge, not a stage inside a
feature. An architectural decision usually outlives the feature that provoked
it and is often consumed by several. A feature cites a merged ADR the way it
cites any other committed contract, and autopilot therefore never has to decide
whether one is required.

### Qualified wave identifiers

Delivery state is laid out as `<state-root>/<wave>/<candidate-id>/...`, in which
the program is **not** a path component. With one program that is harmless;
with two, each program's `W1` names the same directory. The canonical wave
identifier becomes a single lowercase token fusing program and wave with no
separator: `adr046w1`, `spec001w1`. Fusing rather than adding a path component
makes uniqueness intrinsic to the token, so it survives being copied into an
artifact reference, a commit subject, a panel record, or a checkpoint, none of
which have a path structure to lean on, and it requires no state-layout change.

## Consequences

Copilot is the sole installed, selected, and authoritative integration.
The immediate cutover removes `.opencode/`, its manifest, and its command
surface; stale integration state fails closed rather than selecting a
compatibility path. Historical records remain auditable data only.

The legacy form is unaffected and stays valid indefinitely. `--program ADR046
--wave W1` is not deprecated, not warned on, and not on a timer; every
`W0`..`W8` still resolves to its existing directory byte for byte, asserted by
a test rather than by prose. ADR 0046 runs to completion in the legacy form,
because re-addressing a wave would invalidate the candidate digests binding its
existing snapshots, seals, and panel records.

The closed-set guarantee is kept exactly for the legacy namespace and stated
honestly for the new one. A qualified token is a bounded lowercase ASCII
alphanumeric string, so it cannot express a separator, a traversal, an absolute
path, a control character, whitespace, uppercase, or an unbounded length. It is
a strict pattern rather than a nine-element set, and that is the one property
that genuinely widens.

`AGENTS.md` was 122,662 bytes and is injected into every session on every turn
by both harnesses, which made it the largest fixed context cost in the
repository. It is now an index of about 35 KB routing to `docs/contributing/`,
with a byte ratchet in `policy_docs.rs` that fails the next append which would
re-bloat it. No rule was deleted: a prohibition never moves, only its rationale
does.

One PR per wave, merged before the next wave starts, is forced by the delivery
tooling rather than chosen. `seal` requires every item in the current wave to be
merged and the wave exit boundary requires every prior wave to be merged, so
wave N+1 cannot open a panel request until wave N has merged.

If a future CLI honours effort in agent frontmatter or at repo scope, the
indirection here collapses to one line per agent. This record exists partly so
that nobody later mistakes the indirection for a preference.
