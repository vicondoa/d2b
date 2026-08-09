#!/usr/bin/env node
// Coverage for the seat-roster drift guard in check-bindings.mjs, and for the
// input classification this harness depends on.
//
//   node scripts/copilot/test-check-bindings.mjs
//
// Why this exists, and why it is narrow. `make-records.mjs` mirrors the sealed
// roster in `packages/xtask/src/delivery/model.rs` as a plain array, and
// check-bindings.mjs compares the two. That comparison is on the attestation
// path: a helper roster short of the sealed one writes an incomplete record
// set, and a longer one writes a record for a seat the gate does not accept.
// Either way the wave fails at seal time, which is the most expensive place to
// learn about it.
//
// A guard implemented by parsing source with a regex can stop matching without
// anything else changing, and a guard that no longer matches fails open in
// silence. So the guard needs a test that proves it still fires. The baseline
// case is load bearing for the same reason: without it, a fixture that failed
// for an unrelated reason would satisfy every negative case vacuously.
//
// Scope. The roster comparison, plus the required-versus-optional
// classification of the gate's own inputs, which is what keeps the negative
// cases from passing vacuously. The other mirrored constants are scalars
// checked by a shared loop and are not parsed by their own regex; extending the
// harness to them is recorded in .specify/memory/deferred-work.md rather than
// done here, so this stays a test for the guard that was asked for.
//
// It is a plain node script with no test framework because the repository does
// not add tooling for one gate. It runs from `make test-lint`.

import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const here = fileURLToPath(new URL(".", import.meta.url));
const root = join(here, "..", "..");

// The gate under test. It is not in REQUIRED_INPUTS: the classification cases
// below omit one input at a time to measure how the gate reacts, and omitting
// the gate itself measures nothing. Node exits nonzero because the script is
// missing, which it would do even if every check inside had been deleted.
const GATE = "scripts/copilot/check-bindings.mjs";

// Everything check-bindings.mjs reads, as repo-relative paths. Keeping this
// list explicit rather than copying the whole tree keeps a fixture build cheap.
//
// The split is by how the gate behaves when the path is absent, which is the
// only property that decides whether the fixture can safely omit it.
//
// REQUIRED is the set the gate hard-fails on: it either spawns the path, or
// guards it with `existsSync` and calls `fail()`. Copy these unconditionally.
// Add a read of this kind to the gate without listing it here and the baseline
// case fails, so the omission announces itself.
//
// OPTIONAL is the set the gate guards with `existsSync` and then *skips*.
// Omitting one of these does not fail the baseline; the gate simply does not
// run that block, and the fixture silently stops matching the repo. Listing
// them is therefore the only thing that keeps them covered. Copy these when
// they exist, since they are permitted to be absent and an unconditional copy
// would throw ENOENT.
//
// Classify by measuring the gate, not by reading one call site: the
// `.github/skills` scan is itself skip-guarded, but the required record helper
// lives inside that tree, so omitting the directory hard-fails after all. The
// classification cases at the bottom of this file measure every entry, so a
// misfiled path is a test failure rather than a stale comment.
const REQUIRED_INPUTS = [
  ".github/agents",
  ".github/skills",
  "AGENTS.md",
  "tests/AGENTS.md",
  "labs/venus-vulkan-video/AGENTS.md",
  "docs/contributing",
  "third_party/caveman/v1.10.0",
  "tests/tools/tier0-first-pass.sh",
  "packages/d2b-contract-tests/tests/policy_dash_gate.rs",
  "scripts/copilot/prompt-corpus.mjs",
  "scripts/copilot/prompt-corpus-manifest.json",
  "docs/adr/specs/0053-panel-prompt-sources.md",
  "packages/xtask/src/delivery/model.rs",
  "packages/xtask/src/delivery/panel.rs",
  "packages/xtask/src/delivery/mod.rs",
  ".specify/memory",
  ".specify/integration.json",
  ".specify/init-options.json",
  ".specify/integrations/copilot.manifest.json",
];

// What the gate says when each required input is absent. A classification case
// that checked only the exit status would score green on any hard failure,
// including one with nothing to do with the omission, so each entry names the
// diagnostic that ties the failure to the path that was left out.
const REQUIRED_FAILURE_TEXT = {
  ".github/agents": ".github/agents does not exist",
  ".github/skills": "the panel record helper is required",
  "AGENTS.md": "prompt corpus check failed",
  "tests/AGENTS.md": "prompt corpus check failed",
  "labs/venus-vulkan-video/AGENTS.md": "prompt corpus check failed",
  "docs/contributing": "prompt corpus check failed",
  "third_party/caveman/v1.10.0": "Caveman vendor root",
  "tests/tools/tier0-first-pass.sh": "tests/tools/tier0-first-pass.sh is missing",
  "packages/d2b-contract-tests/tests/policy_dash_gate.rs": "policy_dash_gate.rs is missing",
  "scripts/copilot/prompt-corpus.mjs": "prompt-corpus.mjs is missing",
  "scripts/copilot/prompt-corpus-manifest.json": "prompt-corpus-manifest.json is missing",
  "docs/adr/specs/0053-panel-prompt-sources.md": "panel prompt source",
  "packages/xtask/src/delivery/model.rs": "cannot read model.rs",
  "packages/xtask/src/delivery/panel.rs": "cannot read panel.rs",
  "packages/xtask/src/delivery/mod.rs": "cannot read mod.rs",
  ".specify/memory": ".specify/memory/friction-log.md: this register is missing",
  ".specify/integration.json": "does not exist",
  ".specify/init-options.json": "does not exist",
  ".specify/integrations/copilot.manifest.json": "does not exist",
};

const OPTIONAL_INPUTS = [
  ".github/copilot/settings.json",
];

const SELECTION_TABLE = ".github/skills/d2b-panel-round/selection-table.json";
const DISPATCH_POLICY = ".github/skills/d2b-panel-round/dispatch-policy.json";
const CURRENT_PANEL_SEATS = [
  "agentic", "build", "docs", "kernel", "networking", "nixos",
  "observability", "product", "reliability", "security", "simplicity",
  "software", "test",
];

// The marker a register uses to declare that it is empty on purpose. The gate
// compares against its own copy, so spelling it once here means a case cannot
// drift into asserting a string the gate never matches, which would look like
// coverage while testing nothing.
const EMPTY_MARKER = "<!-- d2b-register: intentionally empty -->";

let failures = 0;

// `omit` names one repo-relative input to leave out, which is how the
// classification cases below measure the rule the two lists assert.
function buildFixture(omit) {
  const dir = mkdtempSync(join(tmpdir(), "d2b-check-bindings-"));
  for (const rel of [GATE, ...REQUIRED_INPUTS]) {
    if (rel === omit) continue;
    const dest = join(dir, rel);
    mkdirSync(dirname(dest), { recursive: true });
    cpSync(join(root, rel), dest, { recursive: true });
  }
  for (const rel of OPTIONAL_INPUTS) {
    if (rel === omit) continue;
    if (!existsSync(join(root, rel))) continue;
    const dest = join(dir, rel);
    mkdirSync(dirname(dest), { recursive: true });
    cpSync(join(root, rel), dest, { recursive: true });
  }
  return dir;
}

function run(dir) {
  const r = spawnSync(process.execPath, [join(dir, "scripts", "copilot", "check-bindings.mjs")], {
    encoding: "utf8",
  });
  return { status: r.status, out: `${r.stdout || ""}${r.stderr || ""}` };
}

function mutateIntegration(dir, fn) {
  mutateJson(dir, ".specify/integration.json", fn);
}

function mutateJson(dir, relativePath, fn) {
  const path = join(dir, relativePath);
  const state = JSON.parse(readFileSync(path, "utf8"));
  fn(state);
  writeFileSync(path, `${JSON.stringify(state, null, 2)}\n`);
}

function mutateFile(dir, relativePath, fn) {
  const path = join(dir, relativePath);
  const source = readFileSync(path, "utf8");
  const next = fn(source);
  if (next === source) {
    throw new Error(`fixture: mutation of ${relativePath} was a no-op`);
  }
  writeFileSync(path, next);
}

function removeFile(dir, relativePath) {
  const path = join(dir, relativePath);
  rmSync(path);
}

function mutateSelectionRoster(dir, mutate) {
  mutateJson(dir, SELECTION_TABLE, mutate);
}

function checkCurrentPromptShape(dir) {
  const panelSeats = readdirSync(join(dir, ".github", "agents"))
    .filter((file) => file.startsWith("panel-") && file.endsWith(".agent.md"))
    .map((file) => file.slice("panel-".length, -".agent.md".length))
    .sort();
  if (panelSeats.join(",") !== [...CURRENT_PANEL_SEATS].sort().join(",")) {
    failures += 1;
    console.error(
      `FAIL current panel pool shape: expected [${CURRENT_PANEL_SEATS.join(", ")}], got [${panelSeats.join(", ")}]`,
    );
  }
  const manifest = JSON.parse(
    readFileSync(join(dir, "scripts/copilot/prompt-corpus-manifest.json"), "utf8"),
  );
  const membership = manifest.membership ?? [];
  if (
    membership.length !== 35 ||
    membership.filter((path) => path.startsWith(".github/agents/")).length !== 16 ||
    membership.some((path) => path.endsWith("panel-rust.agent.md")) ||
    !membership.some((path) => path.endsWith("panel-build.agent.md"))
  ) {
    failures += 1;
    console.error(
      `FAIL current prompt corpus shape: expected 35 files and sixteen agent files with build and without current rust`,
    );
  }
}

// Append a row to a memory register in the fixture. Both register cases below
// work by appending rather than by rewriting an existing row, so the case does
// not depend on what the repo's registers happen to contain today.
function appendRegisterRow(dir, reg, row) {
  const path = join(dir, ".specify", "memory", reg);
  const src = readFileSync(path, "utf8");
  writeFileSync(path, `${src.trimEnd()}\n${row}\n`);
}

// Replace a register outright, for the cases that need a table shape the real
// registers do not have.
function writeRegister(dir, reg, text) {
  writeFileSync(join(dir, ".specify", "memory", reg), text);
}

// Rewrite one panel seat's shared finding-bar section inside the fixture. The
// bar is what tells a seat which observations block the round and which belong
// in the summary. The gate requires every selected seat to be byte-identical, because the
// bar was originally restated per seat and silently diverged into ten
// thresholds, three of which were absent entirely. Both mutations below are
// that drift.
function mutateBar(dir, seat, fn) {
  const p = join(dir, ".github", "agents", `panel-${seat}.agent.md`);
  const t = readFileSync(p, "utf8");
  const s = t.indexOf("## The bar for a finding");
  const e = t.indexOf("\n## Output\n", s);
  if (s === -1 || e === -1) {
    throw new Error(`fixture panel-${seat}: no bar section to mutate`);
  }
  const next = fn(t, s, e);
  if (next === t) {
    throw new Error(`fixture panel-${seat}: bar mutation was a no-op, so the case would assert a cause that never occurred`);
  }
  writeFileSync(p, next);
}

function cavemanBlock(dir, agent = "d2b-implementer") {
  const source = readFileSync(join(dir, ".github", "agents", `${agent}.agent.md`), "utf8");
  const start = source.indexOf("<!-- BEGIN D2B-CAVEMAN-COMMUNICATION -->");
  const endMarker = "<!-- END D2B-CAVEMAN-COMMUNICATION -->";
  const end = source.indexOf(endMarker, start);
  if (start < 0 || end < 0) throw new Error("fixture: optional Caveman block is missing");
  return source.slice(start, end + endMarker.length);
}

// A negative case asserts both a nonzero exit and a substring from the roster
// guard itself. Exit status alone would pass if the gate failed for some
// unrelated reason, which is precisely how a guard that no longer fires hides.
const CASES = [
  {
    name: "baseline: an unmutated fixture passes",
    mutate: () => {},
    expectExit: 0,
  },
  {
    name: "dispatch policy agent type drift is rejected",
    mutate: (dir) =>
      mutateJson(dir, DISPATCH_POLICY, (policy) => {
        policy.seats.software.agent_type = "panel-test";
      }),
    expectExit: 1,
    expectText: "dispatch-policy.json seat software agent_type",
  },
  {
    name: "dispatch policy missing communication is rejected",
    mutate: (dir) =>
      mutateJson(dir, DISPATCH_POLICY, (policy) => {
        delete policy.seats.security.communication;
      }),
    expectExit: 1,
    expectText: "dispatch-policy.json seat security must contain",
  },
  {
    name: "dispatch policy seat omission is rejected",
    mutate: (dir) =>
      mutateJson(dir, DISPATCH_POLICY, (policy) => {
        delete policy.seats.build;
      }),
    expectExit: 1,
    expectText: "must contain exactly one binding for every current",
  },
  {
    name: "observed binding field documentation drift is rejected",
    mutate: (dir) =>
      mutateFile(
        dir,
        ".github/skills/d2b-panel-round/SKILL.md",
        (text) => text.replaceAll("`receipt_locator`", "receipt locator"),
      ),
    expectExit: 1,
    expectText: "observed binding documentation is missing receipt_locator",
  },
  {
    name: "schema compatibility documentation drift is rejected",
    mutate: (dir) =>
      mutateFile(
        dir,
        "docs/contributing/panel-review.md",
        (text) => text.replace("Schema-version `3`", "schema version three"),
      ),
    expectExit: 1,
    expectText: "panel-review.md: continuation documentation is missing required text: schema-version `3`",
  },
  {
    name: "atomic continuation publication prose is rejected",
    mutate: (dir) =>
      mutateFile(
        dir,
        "docs/contributing/copilot-agents.md",
        (text) => `${text}\nThe ledger and response are one atomic directory.\n`,
      ),
    expectExit: 1,
    expectText: "still promises atomic ledger/response publication",
  },
  {
    name: "unsupported continuation lifecycle flag is rejected",
    mutate: (dir) =>
      mutateFile(
        dir,
        ".github/skills/d2b-panel-round/SKILL.md",
        (text) =>
          `${text}\n\`\`\`bash\nadvance-verification input output --lifecycle <lifecycle-id>\n\`\`\`\n`,
      ),
    expectExit: 1,
    expectText: "copyable advance-verification command carries unsupported",
  },
  {
    name: "modified admitted Caveman blob is rejected",
    mutate: (dir) =>
      mutateFile(dir, "third_party/caveman/v1.10.0/LICENSE", (text) => `${text}x`),
    expectExit: 1,
    expectText: "hash mismatch",
  },
  {
    name: "missing Caveman license is rejected",
    mutate: (dir) => removeFile(dir, "third_party/caveman/v1.10.0/LICENSE"),
    expectExit: 1,
    expectText: "Caveman vendor file LICENSE is missing",
  },
  {
    name: "extra Caveman runtime file is rejected",
    mutate: (dir) =>
      writeFileSync(
        join(dir, "third_party", "caveman", "v1.10.0", "scripts.py"),
        "runtime\n",
      ),
    expectExit: 1,
    expectText: "outside the closed allowlist",
  },
  {
    name: "changed shell dash admission hash is rejected",
    mutate: (dir) =>
      mutateFile(
        dir,
        "tests/tools/tier0-first-pass.sh",
        (text) => text.replaceAll(
          "5eb826cd03151bcc7cce3f80d40e87733237fedfc6c36d6908aca5fd650a0bdb",
          "0".repeat(64),
        ),
      ),
    expectExit: 1,
    expectText: "tier-0 dash gate is missing required binding text",
  },
  {
    name: "missing optional Caveman agent marker is rejected",
    mutate: (dir) =>
      mutateFile(dir, ".github/agents/panel-build.agent.md", (text) => {
        const start = text.indexOf("<!-- BEGIN D2B-CAVEMAN-COMMUNICATION -->");
        const endMarker = "<!-- END D2B-CAVEMAN-COMMUNICATION -->";
        const end = text.indexOf(endMarker, start) + endMarker.length;
        return `${text.slice(0, start)}${text.slice(end)}`;
      }),
    expectExit: 1,
    expectText: "Caveman-enabled agent set",
  },
  {
    name: "duplicated optional Caveman agent marker is rejected",
    mutate: (dir) =>
      mutateFile(
        dir,
        ".github/agents/panel-build.agent.md",
        (text) => `${text}\n${cavemanBlock(dir, "panel-build")}\n`,
      ),
    expectExit: 1,
    expectText: "must have exactly one start and end marker",
  },
  {
    name: "Caveman marker on architect is rejected",
    mutate: (dir) =>
      mutateFile(
        dir,
        ".github/agents/d2b-architect.agent.md",
        (text) => `${text}\n${cavemanBlock(dir)}\n`,
      ),
    expectExit: 1,
    expectText: "unapproved agent",
  },
  {
    name: "panel verdict JSON drift is rejected",
    mutate: (dir) =>
      mutateFile(
        dir,
        ".github/agents/panel-test.agent.md",
        (text) => text.replace('"signoff": true', '"approved": true'),
      ),
    expectExit: 1,
    expectText: "panel verdict JSON output schema changed",
  },
  {
    name: "panel verification output extension drift is rejected",
    mutate: (dir) =>
      mutateFile(
        dir,
        ".github/agents/panel-test.agent.md",
        (text) => text.replace(
          "During verification, add `verified_issue_statuses`",
          "During verification, omit `verified_issue_statuses`",
        ),
      ),
    expectExit: 1,
    expectText: "panel verification output extension changed",
  },
  {
    name: "integrator staging before finalized evidence and notes is rejected",
    mutate: (dir) =>
      mutateFile(
        dir,
        ".github/agents/d2b-integrator.agent.md",
        (text) => text.replace(
          "Complete both before invoking `stage-diffs.sh`",
          "Complete both after invoking `stage-diffs.sh`",
        ),
      ),
    expectExit: 1,
    expectText: "immutable staging instruction is missing pre-staging input order guidance",
  },
  {
    name: "integrator edits after the completion marker are rejected",
    mutate: (dir) =>
      mutateFile(
        dir,
        ".github/agents/d2b-integrator.agent.md",
        (text) => text.replace(
          "do not edit, replace,",
          "may edit and replace,",
        ),
      ),
    expectExit: 1,
    expectText: "immutable staging instruction is missing immutable completion boundary guidance",
  },
  {
    name: "non-owner direct feature write claim is rejected",
    mutate: (dir) =>
      mutateFile(
        dir,
        ".github/skills/speckit-plan/SKILL.md",
        (text) => text.replace(
          "existing=editor",
          "existing=direct-write",
        ),
      ),
    expectExit: 1,
    expectText: "feature-artifact routing marker is missing or duplicated",
  },
  {
    name: "clarify per-answer integration instruction is rejected",
    mutate: (dir) =>
      mutateFile(dir, ".github/skills/speckit-clarify/SKILL.md", (text) =>
        `${text}\nIntegration after EACH accepted answer (incremental update approach):\n`),
    expectExit: 1,
    expectText: "contradictory direct-write instruction",
  },
  {
    name: "clarify direct checklist save is rejected",
    mutate: (dir) =>
      mutateFile(dir, ".github/skills/speckit-clarify/SKILL.md", (text) =>
        `${text}\nSave the updated checklist file.\n`),
    expectExit: 1,
    expectText: "contradictory direct-write instruction",
  },
  {
    name: "clarify per-write validation is rejected",
    mutate: (dir) =>
      mutateFile(dir, ".github/skills/speckit-clarify/SKILL.md", (text) =>
        `${text}\nValidation (performed after EACH write plus final pass):\n`),
    expectExit: 1,
    expectText: "contradictory direct-write instruction",
  },
  {
    name: "specify unconditional template copy is rejected",
    mutate: (dir) =>
      mutateFile(dir, ".github/skills/speckit-specify/SKILL.md", (text) =>
        `${text}\nCopy the resolved \`spec-template\` file to \`SPECIFY_FEATURE_DIRECTORY/spec.md\` as the starting point\n`),
    expectExit: 1,
    expectText: "contradictory direct-write instruction",
  },
  {
    name: "specify unconditional spec creation is rejected",
    mutate: (dir) =>
      mutateFile(dir, ".github/skills/speckit-specify/SKILL.md", (text) =>
        `${text}\nThe spec directory and file are always created by this command.\n`),
    expectExit: 1,
    expectText: "contradictory direct-write instruction",
  },
  {
    name: "plan direct existing-artifact write is rejected",
    mutate: (dir) =>
      mutateFile(dir, ".github/skills/speckit-plan/SKILL.md", (text) =>
        `${text}\nWrite an existing plan.md directly.\n`),
    expectExit: 1,
    expectText: "contradictory direct-write instruction",
  },
  {
    name: "tasks direct existing-artifact write is rejected",
    mutate: (dir) =>
      mutateFile(dir, ".github/skills/speckit-tasks/SKILL.md", (text) =>
        `${text}\nWrite an existing tasks file directly.\n`),
    expectExit: 1,
    expectText: "contradictory direct-write instruction",
  },
  {
    name: "implement direct task-artifact write is rejected",
    mutate: (dir) =>
      mutateFile(dir, ".github/skills/speckit-implement/SKILL.md", (text) =>
        `${text}\nWrite tasks.md or another existing feature artifact directly.\n`),
    expectExit: 1,
    expectText: "contradictory direct-write instruction",
  },
  {
    name: "converge direct append section is rejected",
    mutate: (dir) =>
      mutateFile(dir, ".github/skills/speckit-converge/SKILL.md", (text) =>
        `${text}\n### 7. Append Convergence Tasks (or report converged)\nAppend to the **end** of \`tasks.md\`.\n`),
    expectExit: 1,
    expectText: "contradictory direct-write instruction",
  },
  {
    name: "checklist direct append wording is rejected",
    mutate: (dir) =>
      mutateFile(dir, ".github/skills/speckit-checklist/SKILL.md", (text) =>
        `${text}\nEach invocation either creates a new file or appends to an existing one.\n`),
    expectExit: 1,
    expectText: "contradictory direct-write instruction",
  },
  {
    name: "analyze manual artifact edit is rejected",
    mutate: (dir) =>
      mutateFile(dir, ".github/skills/speckit-analyze/SKILL.md", (text) =>
        `${text}\nManually edit tasks.md to add coverage for 'performance-metrics'\n`),
    expectExit: 1,
    expectText: "contradictory direct-write instruction",
  },
  {
    name: "editor root escape wording is rejected",
    mutate: (dir) =>
      mutateFile(
        dir,
        ".github/skills/d2b-spec-edit/SKILL.md",
        (text) => text.replace(
          "FEATURE_DIR`: one existing directory under the repository's `specs/`",
          "FEATURE_DIR`: one existing directory under the repository root",
        ),
      ),
    expectExit: 1,
    expectText: "missing fail-closed ownership text",
  },
  {
    name: "prompt corpus membership drift is rejected",
    mutate: (dir) =>
      mutateJson(dir, "scripts/copilot/prompt-corpus-manifest.json", (manifest) => {
        manifest.membership.push("not-in-corpus.md");
      }),
    expectExit: 1,
    expectText: "prompt corpus check failed",
  },
  {
    name: "prompt heading fingerprint drift is rejected",
    mutate: (dir) =>
      mutateFile(dir, "docs/contributing/README.md", (text) =>
        text.replace("# Contributing docs", "# Changed heading")),
    expectExit: 1,
    expectText: "prompt corpus check failed",
  },
  {
    name: "prompt fenced command fingerprint drift is rejected",
    mutate: (dir) =>
      mutateFile(dir, "docs/contributing/gates-and-lints.md", (text) =>
        text.replace("make check-tier0", "make check-tier0 --changed")),
    expectExit: 1,
    expectText: "prompt corpus check failed",
  },
  {
    name: "prompt inline-code fingerprint drift is rejected",
    mutate: (dir) =>
      mutateFile(dir, "AGENTS.md", (text) =>
        text.replace("git+file://$ROOT", "git+file://$ROOT2")),
    expectExit: 1,
    expectText: "prompt corpus check failed",
  },
  {
    name: "prompt link fingerprint drift is rejected",
    mutate: (dir) =>
      mutateFile(dir, "docs/contributing/architecture.md", (text) =>
        text.replace(
          "https://github.com/vicondoa/entrablau.nix",
          "https://github.com/vicondoa/entrablau.nix-bad",
        )),
    expectExit: 1,
    expectText: "prompt corpus check failed",
  },
  {
    name: "prompt number fingerprint drift is rejected",
    mutate: (dir) =>
      mutateFile(dir, "docs/contributing/copilot-agents.md", (text) =>
        text.includes("35 files")
          ? text.replace("35 files", "36 files")
          : text.includes("13 agents")
            ? text.replace("13 agents", "14 agents")
            : `${text}\n36 files\n`),
    expectExit: 1,
    expectText: "prompt corpus check failed",
  },
  {
    name: "prompt normative-token fingerprint drift is rejected",
    mutate: (dir) =>
      mutateFile(dir, "AGENTS.md", (text) =>
        text.replace("MUST", "SHOULD")),
    expectExit: 1,
    expectText: "prompt corpus check failed",
  },
  {
    name: "prompt list hierarchy fingerprint drift is rejected",
    mutate: (dir) =>
      mutateFile(dir, "AGENTS.md", (text) =>
        text.replace("- **Existing code is canon.**", "  - **Existing code is canon.**")),
    expectExit: 1,
    expectText: "prompt corpus check failed",
  },
  {
    name: "prompt table-shape fingerprint drift is rejected",
    mutate: (dir) =>
      mutateFile(dir, "docs/contributing/copilot-agents.md", (text) =>
        text.replace("| --- | --- |", "| --- | --- | --- |")),
    expectExit: 1,
    expectText: "prompt corpus check failed",
  },
  {
    name: "prompt JSON-example fingerprint drift is rejected",
    mutate: (dir) =>
      mutateFile(dir, "docs/contributing/panel-review.md", (text) =>
        text.replace('"engineer": "software"', '"engineer": "tester"')),
    expectExit: 1,
    expectText: "prompt corpus check failed",
  },
  {
    name: "panel prompt source missing current build guidance is rejected",
    mutate: (dir) =>
      mutateFile(
        dir,
        "docs/adr/specs/0053-panel-prompt-sources.md",
        (text) => text.replace(
          "### Build seat source guidance",
          "### Removed seat source guidance",
        ),
      ),
    expectExit: 1,
    expectText: "missing required current guidance",
  },
  {
    name: "operative legacy panel prompt contract is rejected",
    mutate: (dir) =>
      mutateFile(
        dir,
        "docs/adr/specs/0053-panel-prompt-sources.md",
        (text) => `${text}\nThe relevant: false held-reviewer repeated rounds contract remains operative.\n`,
      ),
    expectExit: 1,
    expectText: "keeps an operative",
  },
  {
    name: "an additional installed integration is rejected",
    mutate: (dir) =>
      mutateIntegration(dir, (state) => {
        state.installed_integrations = ["copilot", "other"];
      }),
    expectExit: 1,
    expectText: 'installed_integrations must be exactly ["copilot"]',
  },
  {
    name: "a non-Copilot current integration is rejected",
    mutate: (dir) =>
      mutateIntegration(dir, (state) => {
        state.integration = "other";
      }),
    expectExit: 1,
    expectText: 'integration must be "copilot"',
  },
  {
    name: "a non-Copilot initialization integration is rejected",
    mutate: (dir) => {
      const path = join(dir, ".specify", "init-options.json");
      const options = JSON.parse(readFileSync(path, "utf8"));
      options.integration = "other";
      writeFileSync(path, `${JSON.stringify(options, null, 2)}\n`);
    },
    expectExit: 1,
    expectText: 'init-options.json integration must be "copilot"',
  },
  {
    name: "a retired integration setting is rejected",
    mutate: (dir) =>
      mutateIntegration(dir, (state) => {
        state.integration_settings[["open", "code"].join("")] = {
          script: "sh",
          invoke_separator: ".",
        };
      }),
    expectExit: 1,
    expectText: "contains the retired integration",
  },
  {
    name: "malformed integration state is rejected",
    mutate: (dir) =>
      writeFileSync(
        join(dir, ".specify", "integration.json"),
        '{"integration":\n',
      ),
    expectExit: 1,
    expectText: "is not valid JSON",
  },
  {
    name: "a missing declared Copilot skill is rejected",
    mutate: (dir) =>
      mutateJson(dir, ".specify/integrations/copilot.manifest.json", (manifest) => {
        manifest.files[".github/skills/missing-required-skill/SKILL.md"] = "0".repeat(64);
      }),
    expectExit: 1,
    expectText: "declares required Copilot skill",
  },
  {
    name: "a dangling required Copilot skill symlink is rejected",
    mutate: (dir) => {
      const relativePath = ".github/skills/dangling-required-skill/SKILL.md";
      const path = join(dir, relativePath);
      mkdirSync(dirname(path), { recursive: true });
      symlinkSync("missing-target.md", path);
      mutateJson(dir, ".specify/integrations/copilot.manifest.json", (manifest) => {
        manifest.files[relativePath] = "0".repeat(64);
      });
    },
    expectExit: 1,
    expectText: "does not resolve to a readable regular file",
  },
  {
    name: "a restored retired integration directory is rejected",
    mutate: (dir) => {
      const path = join(dir, ".op" + "encode", "op" + "encode.json");
      mkdirSync(dirname(path), { recursive: true });
      writeFileSync(path, "{}\n");
    },
    expectExit: 1,
    expectText: "retired integration directory is present",
  },
  {
    name: "a restored retired integration manifest is rejected",
    mutate: (dir) => {
      const path = join(
        dir,
        ".specify",
        "integrations",
        "op" + "encode.manifest.json",
      );
      writeFileSync(path, "{}\n");
    },
    expectExit: 1,
    expectText: "retired integration manifest is present",
  },
  {
    name: "a dropped seat is rejected",
    mutate: (dir) => mutateSelectionRoster(dir, (table) => {
      table.optional_seats.pop();
      table.fill_order.pop();
    }),
    expectExit: 1,
    expectText: "exactly thirteen",
  },
  {
    name: "an extra seat is rejected",
    mutate: (dir) => mutateSelectionRoster(dir, (table) => {
      table.optional_seats.push("performance");
      table.fill_order.push("performance");
    }),
    expectExit: 1,
    expectText: "exactly thirteen",
  },
  {
    name: "a reordered roster is rejected",
    mutate: (dir) => mutateSelectionRoster(dir, (table) => {
      [table.mandatory_seats[0], table.mandatory_seats[1]] =
        [table.mandatory_seats[1], table.mandatory_seats[0]];
    }),
    expectExit: 1,
    expectText: "mandatory seat order",
  },
  {
    name: "a selection table the guard cannot parse is rejected rather than skipped",
    mutate: (dir) =>
      writeFileSync(
        join(dir, SELECTION_TABLE),
        '{"artifact_kind":"d2b-panel/selection-table",\n',
      ),
    expectExit: 1,
    expectText: "is not valid JSON",
  },
  {
    name: "selection-table focus drift is rejected",
    mutate: (dir) =>
      mutateSelectionRoster(dir, (table) => {
        table.seats.software.focus = "Different focus";
      }),
    expectExit: 1,
    expectText: "authoritative selection-table focus",
  },
  {
    name: "nixos invariant checklist marker drift is rejected",
    mutate: (dir) =>
      mutateFile(
        dir,
        ".github/agents/panel-nixos.agent.md",
        (text) => text.replace("<!-- panel nixos invariant checklist -->", ""),
      ),
    expectExit: 1,
    expectText: "invariant checklist marker",
  },
  {
    name: "observability invariant checklist marker duplication is rejected",
    mutate: (dir) =>
      mutateFile(
        dir,
        ".github/agents/panel-observability.agent.md",
        (text) => `${text}\n<!-- panel observability invariant checklist -->\n`,
      ),
    expectExit: 1,
    expectText: "invariant checklist marker",
  },
  {
    name: "nixos substantive checklist drift is rejected",
    mutate: (dir) =>
      mutateFile(
        dir,
        ".github/agents/panel-nixos.agent.md",
        (text) => text.replace(
          "The net VM's `10-eth-dhcp` neutralizer",
          "The net VM uplink neutralizer",
        ),
      ),
    expectExit: 1,
    expectText: "substantive repository checklist phrase",
  },
  {
    name: "observability substantive checklist drift is rejected",
    mutate: (dir) =>
      mutateFile(
        dir,
        ".github/agents/panel-observability.agent.md",
        (text) => text.replace(
          "**Unbounded label cardinality.**",
          "Unbounded labels.",
        ),
      ),
    expectExit: 1,
    expectText: "substantive repository checklist phrase",
  },
  {
    name: "networking substantive checklist drift is rejected",
    mutate: (dir) =>
      mutateFile(
        dir,
        ".github/agents/panel-networking.agent.md",
        (text) => text.replace(
          "**MTU and MSS.**",
          "Packet sizing.",
        ),
      ),
    expectExit: 1,
    expectText: "substantive repository checklist phrase",
  },
  {
    name: "kernel substantive checklist drift is rejected",
    mutate: (dir) =>
      mutateFile(
        dir,
        ".github/agents/panel-kernel.agent.md",
        (text) => text.replace(
          "**Process identity races.**",
          "Process identity.",
        ),
      ),
    expectExit: 1,
    expectText: "substantive repository checklist phrase",
  },
  ...[
    ["rust-toolchain.toml", "rust-toolchain.toml"],
    [".cargo/config.toml", ".cargo/config.toml"],
    ["tests/layer1-jobs.json", "tests/layer1-jobs.json"],
    ["tests/test-rust.sh", "tests/test-rust.sh"],
    ["Makefile", "Makefile"],
    ["flake.nix", "flake.nix"],
    ["packages/xtask/src/main.rs", "packages/xtask/src/main.rs"],
    ["packages/xtask/src/delivery/**", "packages/xtask/src/delivery/**"],
    ["tests/static.sh", "tests/static.sh"],
    ["tests/test-lint.sh", "tests/test-lint.sh"],
  ].map(([name, pattern]) => ({
    name: `build trigger ${name} is required`,
    mutate: (dir) =>
      mutateSelectionRoster(dir, (table) => {
        for (const trigger of table.seats.build.triggers) {
          if (trigger.kind === "path") {
            trigger.patterns = trigger.patterns.filter((entry) => entry !== pattern);
          }
        }
      }),
    expectExit: 1,
    expectText: `canonical path ${pattern}`,
  })),
  ...[
    ["network route", "**/*route*"],
    ["network routing", "**/*routing*"],
    ["network mtu", "**/*mtu*"],
    ["network mss", "**/*mss*"],
  ].map(([name, pattern]) => ({
    name: `${name} trigger is required`,
    mutate: (dir) =>
      mutateSelectionRoster(dir, (table) => {
        for (const trigger of table.seats.networking.triggers) {
          if (trigger.kind === "path") {
            trigger.patterns = trigger.patterns.filter((entry) => entry !== pattern);
          }
        }
      }),
    expectExit: 1,
    expectText: `networking triggers are missing canonical path ${pattern}`,
  })),
  {
    name: "a seat missing the shared finding bar is rejected",
    mutate: (dir) => mutateBar(dir, "build", (t, s, e) => t.slice(0, s) + t.slice(e + 1)),
    expectExit: 1,
    expectText: 'no "## The bar for a finding" section',
  },
  {
    name: "a seat whose finding bar diverges from the others is rejected",
    mutate: (dir) =>
      mutateBar(
        dir,
        "build",
        (t, s, e) => `${t.slice(0, e)}\nUse whatever threshold you judge appropriate.\n${t.slice(e)}`,
      ),
    expectExit: 1,
    expectText: "differs from",
  },
  // The registers do not share a column shape. deferred-work.md carries a
  // trailing Ref column, so a guard that reads the last cell reads the ref.
  // The ref below is a legal wave token, which is exactly how that guard
  // passes a row whose disposition is nonsense.
  {
    name: "a bogus disposition is rejected even behind a trailing Ref column",
    mutate: (dir) =>
      appendRegisterRow(
        dir,
        "deferred-work.md",
        "| copilotw6 | test | 2026-07-31 | fixture row | notavocabularyterm | copilotw6 |",
      ),
    expectExit: 1,
    expectText: "is not in the closed set",
  },
  // A statement legitimately quotes a shell pipeline, and the escaped pipe is
  // an extra cell to a naive split. The Recurrence value below is a legal
  // disposition, so a shifted read lands on it and the bogus disposition in
  // the next column goes unseen.
  {
    name: "a bogus disposition is rejected in a row whose statement escapes a pipe",
    mutate: (dir) =>
      appendRegisterRow(
        dir,
        "friction-log.md",
        "| copilotw6 | test | 2026-07-31 | a \\| b | open | notavocabularyterm |",
      ),
    expectExit: 1,
    expectText: "is not in the closed set",
  },
  // The other direction. A cell ending in a literal backslash is written `\\`,
  // and a lookbehind for one backslash refuses to split at the separator that
  // follows, fusing two cells. The row then falls short of the header's
  // disposition index and is skipped without being validated at all.
  {
    name: "a bogus disposition is rejected in a row whose cell ends in a backslash",
    mutate: (dir) =>
      appendRegisterRow(
        dir,
        "friction-log.md",
        "| copilotw6 | test | 2026-07-31 | ends in a backslash \\\\| open | notavocabularyterm |",
      ),
    expectExit: 1,
    expectText: "is not in the closed set",
  },
  // A row narrower than its header was skipped outright by a bare width test,
  // which bypassed every check in the loop rather than just the disposition
  // one. The bogus disposition below is the visible consequence; the bogus
  // wave and category in the same row went unchecked too.
  {
    name: "a row narrower than its header is rejected rather than skipped",
    mutate: (dir) =>
      appendRegisterRow(
        dir,
        "friction-log.md",
        "| copilotw6 | test | notavocabularyterm |",
      ),
    expectExit: 1,
    expectText: "do not line up",
  },
  // The other direction, and the more dangerous one. An unescaped pipe inside
  // a cell adds a column, so the header's disposition index lands one cell to
  // the left. Here it lands on a legal Recurrence value while the real
  // disposition, one column further right, is never looked at.
  {
    name: "a row wider than its header is rejected rather than read off by one",
    mutate: (dir) =>
      appendRegisterRow(
        dir,
        "friction-log.md",
        "| copilotw6 | test | 2026-07-31 | a | b | open | notavocabularyterm |",
      ),
    expectExit: 1,
    expectText: "do not line up",
  },
  // A register written without leading pipes is a valid Markdown table that
  // this parser cannot see. Silently validating none of it is the worst
  // outcome available, so the absence of a header is itself the failure.
  {
    name: "a register with no leading pipes is rejected rather than wholly skipped",
    mutate: (dir) =>
      writeRegister(
        dir,
        "deferred-work.md",
        [
          "Wave | Category | Date | Statement | Disposition | Ref",
          "---|---|---|---|---|---",
          "copilotw6 | test | 2026-07-31 | fixture row | notavocabularyterm | copilotw6",
          "",
        ].join("\n"),
      ),
    expectExit: 1,
    expectText: "no header row was found",
  },
  // Only the first header of a table defines its shape. A later row whose
  // first cell reads Wave is data, and taking it for a second header would
  // skip it unvalidated and let it redefine the columns beneath it.
  {
    name: "a data row that looks like a header is validated, not treated as one",
    mutate: (dir) =>
      appendRegisterRow(
        dir,
        "friction-log.md",
        "| Wave | test | 2026-07-31 | fixture row | open | notavocabularyterm |",
      ),
    expectExit: 1,
    expectText: "is not a legal wave token",
  },
  // An empty cell is not a pass. Each of the three validated columns is
  // mandatory, and a blank one used to short-circuit its own check.
  {
    name: "an empty disposition is rejected rather than skipped",
    mutate: (dir) =>
      appendRegisterRow(
        dir,
        "friction-log.md",
        "| copilotw6 | test | 2026-07-31 | fixture row | open |  |",
      ),
    expectExit: 1,
    expectText: "names no disposition",
  },
  {
    name: "an empty category is rejected rather than skipped",
    mutate: (dir) =>
      appendRegisterRow(
        dir,
        "friction-log.md",
        "| copilotw6 |  | 2026-07-31 | fixture row | open | open |",
      ),
    expectExit: 1,
    expectText: "names no category",
  },
  {
    name: "an empty wave is rejected rather than skipped",
    mutate: (dir) =>
      appendRegisterRow(
        dir,
        "friction-log.md",
        "|  | test | 2026-07-31 | fixture row | open | open |",
      ),
    expectExit: 1,
    expectText: "names no wave",
  },
  // A data row with no header above it has no column to be validated against.
  // The ref below is a legal wave token, so a guard that falls back to the last
  // cell accepts the row and validates nothing.
  {
    name: "a row that precedes any header row is rejected rather than guessed at",
    mutate: (dir) =>
      writeRegister(
        dir,
        "deferred-work.md",
        "| copilotw6 | test | 2026-07-31 | fixture row | notavocabularyterm | copilotw6 |\n",
      ),
    expectExit: 1,
    expectText: "precedes any header row",
  },
  // A header that names no Disposition column cannot validate anything below
  // it, and saying so is the only honest outcome.
  {
    name: "a header with no Disposition column is rejected",
    mutate: (dir) =>
      writeRegister(
        dir,
        "deferred-work.md",
        [
          "| Wave | Category | Date | Statement | Ref |",
          "|---|---|---|---|---|",
          "| copilotw6 | test | 2026-07-31 | fixture row | copilotw6 |",
          "",
        ].join("\n"),
      ),
    expectExit: 1,
    expectText: "names no Disposition",
  },
  // The taxonomy is closed so the three-wave escalation rule can count a
  // category's recurrences. A near-miss spelling groups with nothing.
  {
    name: "a category outside the closed taxonomy is rejected",
    mutate: (dir) =>
      appendRegisterRow(
        dir,
        "friction-log.md",
        "| copilotw6 | testing | 2026-07-31 | fixture row | open | open |",
      ),
    expectExit: 1,
    expectText: "is not in the closed taxonomy",
  },
  // A row is found by its leading pipe, so a row that lost one reads as prose
  // and every column on it goes unvalidated. The disposition below is bogus:
  // if the line were read as a row at all, the gate would reject it for that
  // instead, so the expected diagnostic distinguishes the two outcomes.
  {
    name: "a row that lost its leading pipe is rejected rather than read as prose",
    mutate: (dir) =>
      appendRegisterRow(
        dir,
        "friction-log.md",
        " copilotw6 | test | 2026-07-31 | fixture row | open | notavocabularyterm |",
      ),
    expectExit: 1,
    expectText: "lost its leading pipe",
  },
  // A row that lost both outer pipes is still a row when it sits between two
  // rows of a table, because a line there is not prose. The disposition is
  // bogus so a guard that read the line as a row would reject it for that.
  {
    name: "a row that lost both outer pipes inside a table is rejected",
    mutate: (dir) => {
      const path = join(dir, ".specify", "memory", "friction-log.md");
      const src = readFileSync(path, "utf8").trimEnd().split("\n");
      src.splice(
        src.length - 1,
        0,
        "copilotw6 | test | 2026-07-31 | fixture row | open | notavocabularyterm",
      );
      writeFileSync(path, `${src.join("\n")}\n`);
    },
    expectExit: 1,
    expectText: "lost its leading pipe",
  },
  // The counterpart: an ordinary pipe in prose is not a lost row. Escaping one
  // to satisfy the gate would corrupt the text it appears in, so a sentence and
  // a shell pipeline clear of the table must both pass.
  {
    name: "a pipe in prose clear of the table is not read as a lost row",
    mutate: (dir) => {
      const path = join(dir, ".specify", "memory", "friction-log.md");
      const src = readFileSync(path, "utf8");
      writeFileSync(
        path,
        `${src.trimEnd()}\n\nRun systemctl list-units | grep d2b | wc -l to count them.\n`,
      );
    },
    expectExit: 0,
  },
  // A fenced example may hold anything, including a line shaped like a row.
  {
    name: "a fenced block holding a row-shaped line is not read as a lost row",
    mutate: (dir) => {
      const path = join(dir, ".specify", "memory", "friction-log.md");
      const src = readFileSync(path, "utf8");
      writeFileSync(
        path,
        `${src.trimEnd()}\n\n\`\`\`\ncopilotw6 | test | 2026-07-31 | x | open | bogus |\n\`\`\`\n`,
      );
    },
    expectExit: 0,
  },
  {
    // The first arm of the predicate, exercised alone. A blank line closes the
    // table above, so dispositionIdx is -1 and the second arm cannot fire. What
    // is left is the shape of a register row: more than one unescaped pipe,
    // ending in one.
    name: "a row that lost its leading pipe is rejected even with no table open",
    mutate: (dir) => {
      const path = join(dir, ".specify", "memory", "friction-log.md");
      const src = readFileSync(path, "utf8");
      writeFileSync(
        path,
        `${src.trimEnd()}\n\ncopilotw6 | test | 2026-07-31 | x | 1 | open |\n`,
      );
    },
    expectExit: 1,
    expectText: "lost its leading pipe",
  },
  {
    // The pipes > 1 boundary. One unescaped pipe, ending in it, with no table
    // open: too few pipes for the first arm, no table for the second.
    name: "a single trailing pipe with no table open is not read as a lost row",
    mutate: (dir) => {
      const path = join(dir, ".specify", "memory", "friction-log.md");
      const src = readFileSync(path, "utf8");
      writeFileSync(path, `${src.trimEnd()}\n\nSee the note below |\n`);
    },
    expectExit: 0,
  },
  {
    // A fence opened between two rows of a table swallows the rest of it. The
    // gate reported nothing before: the rows vanished from the count and it
    // exited 0.
    name: "a fence opened inside a table is rejected rather than swallowing its rows",
    mutate: (dir) => {
      const path = join(dir, ".specify", "memory", "friction-log.md");
      const src = readFileSync(path, "utf8").trimEnd().split("\n");
      src.splice(src.length - 1, 0, "```", "swallowed", "```");
      writeFileSync(path, `${src.join("\n")}\n`);
    },
    expectExit: 1,
    expectText: "opens inside a table",
  },
  {
    // A register emptied of every row still had a header, so it passed and the
    // success count simply went down. Nothing compares that count to anything.
    name: "a register with a header and no data rows is rejected",
    mutate: (dir) => {
      const path = join(dir, ".specify", "memory", "engineering-debt.md");
      const src = readFileSync(path, "utf8").split("\n");
      const kept = [];
      let seen = 0;
      for (const line of src) {
        if (line.trim().startsWith("|")) {
          seen += 1;
          if (seen > 2) continue;
        }
        kept.push(line);
      }
      writeFileSync(path, kept.join("\n"));
    },
    expectExit: 1,
    expectText: "not one data row",
  },
  {
    // Security and product both said the zero-rows message promised a remedy
    // that did not work. The marker is that remedy, and it has to actually
    // pass.
    name: "a register declared intentionally empty passes with no rows",
    mutate: (dir) => {
      const path = join(dir, ".specify", "memory", "engineering-debt.md");
      const src = readFileSync(path, "utf8").split("\n");
      const kept = [];
      let seen = 0;
      for (const line of src) {
        if (line.trim().startsWith("|")) {
          seen += 1;
          if (seen > 2) continue;
        }
        kept.push(line);
      }
      kept.push("", EMPTY_MARKER, "");
      writeFileSync(path, kept.join("\n"));
    },
    expectExit: 0,
  },
  {
    // A marker left behind once rows return would licence the next truncation
    // silently, so it is refused rather than ignored.
    name: "a register that declares itself empty and has rows is rejected",
    mutate: (dir) => {
      const path = join(dir, ".specify", "memory", "engineering-debt.md");
      const src = readFileSync(path, "utf8").trimEnd();
      writeFileSync(path, `${src}\n\n${EMPTY_MARKER}\n`);
    },
    expectExit: 1,
    expectText: "declares itself intentionally empty and has",
  },
  {
    // The marker is matched on a trimmed line, so an author who indents it or
    // leaves trailing space still gets the behaviour the message describes.
    // Without this, the marker would be usable only when written flush left,
    // which the diagnostic does not say.
    name: "an intentionally-empty marker is recognized around surrounding whitespace",
    mutate: (dir) => {
      const path = join(dir, ".specify", "memory", "engineering-debt.md");
      const src = readFileSync(path, "utf8").split("\n");
      const kept = [];
      let seen = 0;
      for (const line of src) {
        if (line.trim().startsWith("|")) {
          seen += 1;
          if (seen > 2) continue;
        }
        kept.push(line);
      }
      kept.push("", `   ${EMPTY_MARKER}\t `, "");
      writeRegister(dir, "engineering-debt.md", kept.join("\n"));
    },
    expectExit: 0,
  },
  {
    // A fence suppresses every line inside it, and the marker must be no
    // exception. If it were honoured inside a fence, a register could be
    // emptied and excused by a marker sitting in an example block that the
    // author never meant as a declaration.
    name: "an intentionally-empty marker inside a fence does not excuse an empty register",
    mutate: (dir) => {
      const path = join(dir, ".specify", "memory", "engineering-debt.md");
      const src = readFileSync(path, "utf8").split("\n");
      const kept = [];
      let seen = 0;
      for (const line of src) {
        if (line.trim().startsWith("|")) {
          seen += 1;
          if (seen > 2) continue;
        }
        kept.push(line);
      }
      kept.push("", "```", EMPTY_MARKER, "```", "");
      writeRegister(dir, "engineering-debt.md", kept.join("\n"));
    },
    expectExit: 1,
    expectText: "not one data row",
  },
  {
    // The marker excuses an absent row, never an absent table. A register with
    // no header row cannot be validated at all, so the missing header is
    // reported ahead of the row count and the marker does not reach it.
    name: "an intentionally-empty marker does not excuse a register with no header",
    mutate: (dir) => {
      writeRegister(
        dir,
        "engineering-debt.md",
        `# Engineering debt\n\n${EMPTY_MARKER}\n`,
      );
    },
    expectExit: 1,
    expectText: "no header row",
  },
  {
    // Declaring the same thing twice is still declaring it once. A second
    // marker is not an error, so an author who moves the marker rather than
    // deleting the old one is not blocked by a diagnostic about a defect that
    // does not exist.
    name: "a repeated intentionally-empty marker is not itself an error",
    mutate: (dir) => {
      const path = join(dir, ".specify", "memory", "engineering-debt.md");
      const src = readFileSync(path, "utf8").split("\n");
      const kept = [];
      let seen = 0;
      for (const line of src) {
        if (line.trim().startsWith("|")) {
          seen += 1;
          if (seen > 2) continue;
        }
        kept.push(line);
      }
      kept.push("", EMPTY_MARKER, "", EMPTY_MARKER, "");
      writeRegister(dir, "engineering-debt.md", kept.join("\n"));
    },
    expectExit: 0,
  },
  {
    // An unterminated fence in a long register is hard to find without the
    // line it opened on. The register is written here rather than appended to
    // the fixture's own, so the expected line number is a property of this
    // case and not of however long the repository's register happens to be.
    // Relaxing the assertion to just "opened on line" would still discriminate
    // against the message that carried no number, but it would no longer catch
    // the off-by-one this exists to pin.
    name: "an unterminated fence names the line it opened on",
    mutate: (dir) => {
      writeRegister(
        dir,
        "friction-log.md",
        [
          "# Friction log", // 1
          "", // 2
          "| Wave | Category | Date | Statement | Recurrence | Disposition |", // 3
          "|---|---|---|---|---|---|", // 4
          "| copilotw6 | test | 2026-07-31 | placeholder | 1 | open |", // 5
          "", // 6
          "```", // 7
          "still open at EOF", // 8
          "",
        ].join("\n"),
      );
    },
    expectExit: 1,
    expectText: "opened on line 7",
  },
  {
    // An unterminated fence swallows every line after it. A register whose
    // table sits below one would be skipped entirely while an earlier table
    // kept sawHeader true, so the gate has to notice the fence itself.
    name: "an unterminated fence is rejected rather than swallowing the rest of the file",
    mutate: (dir) => {
      const path = join(dir, ".specify", "memory", "friction-log.md");
      const src = readFileSync(path, "utf8");
      writeFileSync(path, `${src.trimEnd()}\n\n\`\`\`\nstill open at EOF\n`);
    },
    expectExit: 1,
    expectText: "never closed",
  },
  {
    // A register whose lines end with a lone CR collapses to one line, so not
    // one row in it is read as a row. The count is what shows it: the rows are
    // simply absent from the total.
    name: "a register written with lone CR line endings still has its rows read",
    mutate: (dir) => {
      const path = join(dir, ".specify", "memory", "friction-log.md");
      const src = readFileSync(path, "utf8");
      writeFileSync(
        path,
        `${src.trimEnd()}\n| copilotw6 | test | 2026-07-31 | x | 1 | bogus |\n`
          .replace(/\n/g, "\r"),
      );
    },
    expectExit: 1,
    expectText: "bogus",
  },
  {
    name: "a register path that is a directory fails cleanly rather than crashing",
    mutate: (dir) => {
      const path = join(dir, ".specify", "memory", "friction-log.md");
      rmSync(path);
      mkdirSync(path);
    },
    expectExit: 1,
    expectText: "is not a regular file",
  },
];

// Does the classification above match what the gate actually does?
//
// A comment is not a test. The gate could stop hard-failing on a required read,
// or start hard-failing on an optional one, and the comment would keep reading
// true while the list was wrong. Both directions are defects. A required entry
// that has quietly become skippable makes every negative case above vacuous,
// because the fixture is still built with it present. An optional entry listed
// as required throws ENOENT the day the repo legitimately drops that path,
// before a single case runs.
//
// So measure it rather than asserting it in prose. Omit exactly one input,
// run the gate, and check the exit status the classification predicts. The
// baseline case establishes that a complete fixture exits 0, so a nonzero exit
// here is caused by the omission rather than by the fixture being broken to
// begin with. That is not the same as knowing which failure it is: the gate
// has many, and any of them produces the same status. A required case
// therefore also names the diagnostic it expects, so a gate that hard-fails
// for some other reason cannot be read as evidence of this one.
function classificationCases() {
  const cases = [];
  for (const rel of REQUIRED_INPUTS) {
    if (!REQUIRED_FAILURE_TEXT[rel]) {
      // Without this the case would fall back to a status-only assertion,
      // which is the weaker check this table exists to replace.
      console.error(`error: no expected diagnostic recorded for required input ${rel}`);
      failures += 1;
      continue;
    }
    cases.push({
      name: `classification: omitting required ${rel} fails the gate`,
      omit: rel,
      expectNonZero: true,
      expectText: REQUIRED_FAILURE_TEXT[rel],
    });
  }
  for (const rel of OPTIONAL_INPUTS) {
    if (!existsSync(join(root, rel))) {
      // The repo does not ship this path, so the fixture never copies it and
      // the baseline already runs the gate without it. There is nothing to omit,
      // and reporting the skip keeps that visible rather than counting a case
      // that measured nothing.
      cases.push({ name: `classification: optional ${rel} is not in the repo`, skip: true });
      continue;
    }
    cases.push({
      name: `classification: omitting optional ${rel} still passes`,
      omit: rel,
      expectExit: 0,
    });
  }
  return cases;
}

const ALL_CASES = [...CASES, ...classificationCases()];

let ran = 0;
let skipped = 0;

for (const c of ALL_CASES) {
  if (c.skip) {
    console.log(`skip ${c.name}`);
    skipped += 1;
    continue;
  }
  const dir = buildFixture(c.omit);
  try {
    if (c.mutate) c.mutate(dir);
    if (c.name.startsWith("baseline:")) checkCurrentPromptShape(dir);
    const { status, out } = run(dir);
    ran += 1;
    if (c.expectNonZero && status === 0) {
      failures += 1;
      console.error(`FAIL ${c.name}: expected a nonzero exit, got 0`);
      console.error(out.split("\n").slice(0, 20).join("\n"));
    } else if (!c.expectNonZero && status !== c.expectExit) {
      failures += 1;
      console.error(`FAIL ${c.name}: expected exit ${c.expectExit}, got ${status}`);
      console.error(out.split("\n").slice(0, 20).join("\n"));
    } else if (c.expectText && !out.includes(c.expectText)) {
      failures += 1;
      console.error(
        `FAIL ${c.name}: exited ${status} as expected but the output does not ` +
        `mention ${JSON.stringify(c.expectText)}, so it failed for another reason`,
      );
      console.error(out.split("\n").slice(0, 20).join("\n"));
    } else {
      console.log(`ok   ${c.name}`);
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

// Report what did not run alongside what did. A case that stops running is
// indistinguishable from one that passes if only the passing count is printed,
// and a silently skipped check is the defect class this harness exists to
// catch.
const tally = `${ran} of ${ALL_CASES.length} cases, ${skipped} skipped`;
if (failures) {
  console.error(`\ncheck-bindings guard: ${failures} failed (${tally})`);
  process.exit(1);
}
console.log(`\ncheck-bindings guard: ${tally}, all passed`);
