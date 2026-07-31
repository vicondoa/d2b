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

import { cpSync, mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const here = fileURLToPath(new URL(".", import.meta.url));
const root = join(here, "..", "..");

// Everything check-bindings.mjs reads, as repo-relative paths. Keeping this
// list explicit rather than copying the whole tree keeps a fixture build cheap
// and makes a new input announce itself: add a read to the gate without adding
// it here and the baseline case fails.
const INPUTS = [
  "scripts/copilot/check-bindings.mjs",
  ".github/agents",
  ".github/skills",
  ".specify/integration.json",
  ".specify/memory",
  "packages/xtask/src/delivery/model.rs",
  "packages/xtask/src/delivery/panel.rs",
  "packages/xtask/src/delivery/mod.rs",
];

const HELPER = ".github/skills/d2b-panel-round/scripts/make-records.mjs";

let failures = 0;

function buildFixture() {
  const dir = mkdtempSync(join(tmpdir(), "d2b-check-bindings-"));
  for (const rel of INPUTS) {
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
function setRolesBlock(dir, text) {
  const path = join(dir, HELPER);
  const src = readFileSync(path, "utf8");
  const next = src.replace(/const\s+ROLES\s*=\s*\[[\s\S]*?\];/, text);
  if (next === src) {
    throw new Error("fixture: ROLES declaration not found in make-records.mjs");
  }
  writeFileSync(path, next);
}

function rolesLiteral(roles) {
  return `const ROLES = [\n  ${roles.map((r) => `"${r}"`).join(", ")},\n];`;
}

const SEALED = [
  "software", "test", "nixos", "networking", "security",
  "rust", "product", "docs", "observability", "kernel",
];

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
    mutate: (dir) => setRolesBlock(dir, rolesLiteral(SEALED.filter((r) => r !== "kernel"))),
    expectExit: 1,
    expectText: "make-records.mjs ROLES is [",
  },
  {
    name: "an extra seat is rejected",
    mutate: (dir) => setRolesBlock(dir, rolesLiteral([...SEALED, "performance"])),
    expectExit: 1,
    expectText: "make-records.mjs ROLES is [",
  },
  {
    name: "a reordered roster is rejected",
    mutate: (dir) => {
      const swapped = [...SEALED];
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
