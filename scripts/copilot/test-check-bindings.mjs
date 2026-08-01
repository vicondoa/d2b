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

import { cpSync, existsSync, mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
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
  "packages/xtask/src/delivery/model.rs",
  "packages/xtask/src/delivery/panel.rs",
  "packages/xtask/src/delivery/mod.rs",
  ".specify/memory",
];

// What the gate says when each required input is absent. A classification case
// that checked only the exit status would score green on any hard failure,
// including one with nothing to do with the omission, so each entry names the
// diagnostic that ties the failure to the path that was left out.
const REQUIRED_FAILURE_TEXT = {
  ".github/agents": ".github/agents does not exist",
  ".github/skills": "the panel record helper is required",
  "packages/xtask/src/delivery/model.rs": "cannot read model.rs",
  "packages/xtask/src/delivery/panel.rs": "cannot read panel.rs",
  "packages/xtask/src/delivery/mod.rs": "cannot read mod.rs",
  ".specify/memory": ".specify/memory/friction-log.md: this register is missing",
};

const OPTIONAL_INPUTS = [
  ".github/copilot/settings.json",
  ".specify/integration.json",
];

const HELPER = ".github/skills/d2b-panel-round/scripts/make-records.mjs";

// The regex the gate itself uses to find the roster. Sharing the shape is
// deliberate: if the gate can parse the declaration, so can the harness, and
// if it cannot, both fail rather than one silently disagreeing.
const ROLES_DECL = /const\s+ROLES\s*=\s*\[[\s\S]*?\];/;

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

// Replace the helper's ROLES declaration with arbitrary text. Taking the whole
// declaration lets a case rewrite it into a shape the guard's regex cannot
// parse, which is the drift a refactor actually produces.
//
// The rewrite must actually change the file. A mutation that silently produced
// the original text would leave the fixture unmutated, the gate would exit 0,
// and the case would report a failure whose stated cause is wrong. Assert it
// instead, so a no-op mutation names itself.
function setRolesBlock(dir, text) {
  const path = join(dir, HELPER);
  const src = readFileSync(path, "utf8");
  if (!ROLES_DECL.test(src)) {
    throw new Error("fixture: ROLES declaration not found in make-records.mjs");
  }
  const next = src.replace(ROLES_DECL, text);
  if (next === src) {
    throw new Error("fixture: the mutation did not change make-records.mjs");
  }
  writeFileSync(path, next);
}

// The roster the negative cases perturb is read out of the fixture rather than
// written down here.
//
// Writing it down would make a third copy, alongside `model.rs` and
// `make-records.mjs`, and drift between copies is the exact class the guard
// under test exists to catch. Nothing in this suite would notice such a drift:
// the baseline case mutates nothing, so it never evaluates the array at all,
// and the negative cases still pass, because perturbing a stale roster also
// mismatches `model.rs`. The suite would stay green while testing a roster the
// repo had stopped using.
//
// Deriving it removes the third copy instead of documenting it.
function rosterFromFixture(dir) {
  const src = readFileSync(join(dir, HELPER), "utf8");
  const block = src.match(ROLES_DECL);
  if (!block) {
    throw new Error("fixture: cannot read the roster from make-records.mjs");
  }
  const roles = [...block[0].matchAll(/"([^"]+)"/g)].map((m) => m[1]);
  if (roles.length < 2) {
    throw new Error(`fixture: roster has ${roles.length} seats; cannot perturb it`);
  }
  return roles;
}

function rolesLiteral(roles) {
  return `const ROLES = [\n  ${roles.map((r) => `"${r}"`).join(", ")},\n];`;
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
    name: "a dropped seat is rejected",
    mutate: (dir) => {
      const roster = rosterFromFixture(dir);
      setRolesBlock(dir, rolesLiteral(roster.slice(0, -1)));
    },
    expectExit: 1,
    expectText: "make-records.mjs ROLES is [",
  },
  {
    name: "an extra seat is rejected",
    mutate: (dir) => {
      const roster = rosterFromFixture(dir);
      setRolesBlock(dir, rolesLiteral([...roster, "performance"]));
    },
    expectExit: 1,
    expectText: "make-records.mjs ROLES is [",
  },
  {
    name: "a reordered roster is rejected",
    mutate: (dir) => {
      const swapped = rosterFromFixture(dir);
      [swapped[0], swapped[1]] = [swapped[1], swapped[0]];
      setRolesBlock(dir, rolesLiteral(swapped));
    },
    expectExit: 1,
    expectText: "make-records.mjs ROLES is [",
  },
  {
    name: "a roster the guard cannot parse is rejected rather than skipped",
    mutate: (dir) => setRolesBlock(dir, "const ROLES = PANEL_SEATS.slice();"),
    expectExit: 1,
    expectText: "cannot parse ROLES",
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
    expectText: "carries a pipe but does not",
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
// here is attributable to the omission and not to unrelated breakage. A
// required case additionally names the diagnostic it expects, so a gate that
// hard-fails for some other reason cannot be read as evidence of this one.
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
