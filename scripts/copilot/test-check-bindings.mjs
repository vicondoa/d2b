#!/usr/bin/env node
// Coverage for the seat-roster drift guard in check-bindings.mjs.
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
// Scope. This covers the roster comparison only. The other mirrored constants
// are scalars checked by a shared loop and are not parsed by their own regex;
// extending the harness to them is recorded in .specify/memory/deferred-work.md
// rather than done here, so this stays a test for the guard that was asked for.
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
// lives inside that tree, so omitting the directory hard-fails after all.
const REQUIRED_INPUTS = [
  "scripts/copilot/check-bindings.mjs",
  ".github/agents",
  ".github/skills",
  "packages/xtask/src/delivery/model.rs",
  "packages/xtask/src/delivery/panel.rs",
  "packages/xtask/src/delivery/mod.rs",
];

const OPTIONAL_INPUTS = [
  ".github/copilot/settings.json",
  ".specify/integration.json",
  ".specify/memory",
];

const HELPER = ".github/skills/d2b-panel-round/scripts/make-records.mjs";

// The regex the gate itself uses to find the roster. Sharing the shape is
// deliberate: if the gate can parse the declaration, so can the harness, and
// if it cannot, both fail rather than one silently disagreeing.
const ROLES_DECL = /const\s+ROLES\s*=\s*\[[\s\S]*?\];/;

let failures = 0;

function buildFixture() {
  const dir = mkdtempSync(join(tmpdir(), "d2b-check-bindings-"));
  for (const rel of REQUIRED_INPUTS) {
    const dest = join(dir, rel);
    mkdirSync(dirname(dest), { recursive: true });
    cpSync(join(root, rel), dest, { recursive: true });
  }
  for (const rel of OPTIONAL_INPUTS) {
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
];

for (const c of CASES) {
  const dir = buildFixture();
  try {
    c.mutate(dir);
    const { status, out } = run(dir);
    if (status !== c.expectExit) {
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

if (failures) {
  console.error(`\ncheck-bindings roster guard: ${failures} of ${CASES.length} cases failed`);
  process.exit(1);
}
console.log(`\ncheck-bindings roster guard: ${CASES.length} cases passed`);
