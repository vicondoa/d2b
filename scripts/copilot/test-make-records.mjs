#!/usr/bin/env node
// Coverage for make-records.mjs, the helper that turns ten reviewer verdicts
// into the records `delivery wave panel-attest` consumes.
//
//   node scripts/copilot/test-make-records.mjs
//
// This script produces the artifacts that seal a wave, so its fail-closed
// behaviour is the thing under test. Each case builds a complete, valid round
// directory and then breaks exactly one thing, which is what makes a failure
// point at a cause rather than at the fixture.
//
// It is a plain node script with no test framework because the repository
// does not add tooling for one gate. It runs from `make test-lint`.

import { mkdtempSync, mkdirSync, writeFileSync, rmSync, existsSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const here = fileURLToPath(new URL(".", import.meta.url));
const root = join(here, "..", "..");
const script = join(root, ".github", "skills", "d2b-panel-round", "scripts", "make-records.mjs");

const ROLES = [
  "software", "test", "nixos", "networking", "security",
  "rust", "product", "docs", "observability", "kernel",
];

let failures = 0;
const check = (name, ok, detail) => {
  if (ok) {
    console.log(`  ok   ${name}`);
  } else {
    failures += 1;
    console.error(`  FAIL ${name}${detail ? `: ${detail}` : ""}`);
  }
};

// Build a complete, valid round directory, then apply a mutation.
function buildRound(mutate) {
  const dir = mkdtempSync(join(tmpdir(), "d2b-panel-"));
  mkdirSync(join(dir, "verdicts"), { recursive: true });

  const state = {
    address: {
      round: "spec001w1r1",
      base: "a".repeat(40),
      previous_tip: "b".repeat(40),
      tip: "c".repeat(40),
      delta_sha256: "d".repeat(64),
      full_sha256: "e".repeat(64),
    },
    candidate: {
      candidate_id: "cand-0001",
      content_id: "content-0001",
      snapshot_sha256: "f".repeat(64),
      program: "SPEC001",
      wave: "spec001w1",
    },
    observed: Object.fromEntries(ROLES.map((r, i) => [r, {
      model: "gemini-3.1-pro-preview",
      reasoning_effort: "high",
      run_id: `run-${i}`,
      receipt_locator: `github-copilot://receipt/${i}`,
    }])),
    verdicts: Object.fromEntries(ROLES.map((r) => [r, {
      engineer: r,
      signoff: true,
      summary: `${r} seat reviewed the delta.`,
      recommendations: [],
    }])),
  };

  if (mutate) mutate(state);

  writeFileSync(join(dir, "address.json"), JSON.stringify(state.address, null, 2));
  writeFileSync(join(dir, "candidate.json"), JSON.stringify(state.candidate, null, 2));
  writeFileSync(join(dir, "observed.json"), JSON.stringify(state.observed, null, 2));
  for (const [role, v] of Object.entries(state.verdicts)) {
    writeFileSync(join(dir, "verdicts", `${role}.json`), JSON.stringify(v, null, 2));
  }
  return dir;
}

function run(dir) {
  try {
    const stdout = execFileSync("node", [script, dir], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
    return { code: 0, out: stdout, err: "" };
  } catch (e) {
    return { code: e.status ?? 1, out: e.stdout ?? "", err: e.stderr ?? "" };
  }
}

// A case that must be REJECTED, and whose message must name the cause.
function rejects(name, mutate, expect) {
  const dir = buildRound(mutate);
  try {
    const r = run(dir);
    const text = `${r.out}${r.err}`;
    if (r.code === 0) {
      check(name, false, "exited 0; a malformed round must fail closed");
      return;
    }
    check(name, expect.test(text), `message did not match ${expect}: ${text.trim().slice(0, 200)}`);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

console.log("make-records: the happy path");
{
  const dir = buildRound();
  try {
    const r = run(dir);
    check("a complete unanimous round is accepted", r.code === 0, `${r.err}`);
    const recordsDir = join(dir, "records");
    check("one record per seat is written", ROLES.every((x) => existsSync(join(recordsDir, `${x}.json`))));
    check("no temp file survives the write", !ROLES.some((x) => existsSync(join(recordsDir, `${x}.json.tmp`))));
    if (existsSync(join(recordsDir, "security.json"))) {
      const rec = JSON.parse(readFileSync(join(recordsDir, "security.json"), "utf8"));
      check("the record carries the observed effort", rec.reasoning_effort === "high", JSON.stringify(rec.reasoning_effort));
      check("the record carries the candidate address", rec.candidate_id === "cand-0001");
      check("the record digests the verdict", typeof rec.output_sha256 === "string" && rec.output_sha256.length === 64);
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

console.log("make-records: a structured finding reaches the seal as a string");
{
  // The shared finding bar asks each seat for an object. `PanelRecord` in
  // packages/xtask/src/delivery/panel.rs is `Vec<String>`, so an object
  // written through verbatim passes every check here and then fails
  // deserialization at the seal. This case is the guard against that.
  const dir = buildRound((s) => {
    s.verdicts.rust.signoff = false;
    s.verdicts.rust.recommendations = [{
      severity: "critical",
      where: "packages/d2b-core/src/lib.rs:1",
      what: "the thing is wrong",
      why: "it breaks the contract",
      fix: "stop doing that",
    }];
  });
  try {
    const r = run(dir);
    // Exit 3 is the designed non-unanimous verdict: the records are written,
    // the round does not pass. The point of this case is the record contents.
    check("a round carrying an object finding still writes records", r.code === 3, `exit ${r.code}: ${r.err}`);
    const p = join(dir, "records", "rust.json");
    if (existsSync(p)) {
      const rec = JSON.parse(readFileSync(p, "utf8"));
      const got = rec.recommendations[0];
      check(
        "an object finding is rendered to a string, not written through as an object",
        typeof got === "string",
        `recommendations[0] is ${typeof got}: ${JSON.stringify(got)}`,
      );
      check(
        "the rendered finding keeps its severity",
        typeof got === "string" && got.includes("critical"),
        JSON.stringify(got),
      );
      check(
        "the rendered finding keeps its location and fix",
        typeof got === "string" && got.includes("lib.rs:1") && got.includes("stop doing that"),
        JSON.stringify(got),
      );
    } else {
      check("a record was written for the seat carrying the object finding", false, "records/rust.json missing");
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

console.log("make-records: attestation integrity, which is why this script exists");
rejects(
  "a lane that ran at the wrong effort cannot be attested",
  (s) => { s.observed.security.reasoning_effort = "medium"; },
  /effort "medium"|policy pins/i,
);
rejects(
  "a lane that ran on the wrong model cannot be attested",
  (s) => { s.observed.rust.model = "claude-opus-5"; },
  /claude-opus-5|policy pins/i,
);
rejects(
  "a seat with no observed binding cannot be attested",
  (s) => { delete s.observed.docs; },
  /observed\.json has no entry|docs/i,
);
rejects(
  "a missing observed field is not defaulted",
  (s) => { delete s.observed.kernel.run_id; },
  /run_id/i,
);
rejects(
  "a receipt locator without its provider scheme is rejected",
  (s) => { s.observed.product.receipt_locator = "receipt/7"; },
  /receipt_locator/i,
);

console.log("make-records: the verdict contract");
rejects(
  "signoff true with findings is rejected",
  (s) => { s.verdicts.test.recommendations = ["something is wrong"]; },
  /signoff|partial pass/i,
);
rejects(
  "signoff false with no findings is rejected",
  (s) => { s.verdicts.test.signoff = false; },
  /signoff|partial pass/i,
);
rejects(
  "a missing seat fails the round",
  (s) => { delete s.verdicts.observability; },
  /observability|missing|ten/i,
);
rejects(
  "an unknown seat is rejected",
  (s) => { s.verdicts.performance = { engineer: "performance", signoff: true, summary: "x", recommendations: [] }; },
  /performance|roster|unknown/i,
);
rejects(
  "a verdict whose engineer disagrees with its filename is rejected",
  (s) => { s.verdicts.nixos.engineer = "networking"; },
  /engineer|nixos|networking/i,
);
rejects(
  "an empty summary is rejected",
  (s) => { s.verdicts.software.summary = "   "; },
  /summary/i,
);

console.log("make-records: bounds, so a record stays a verdict and not a transcript");
rejects(
  "an oversized summary is rejected",
  (s) => { s.verdicts.software.summary = "x".repeat(5000); },
  /summary is \d+ characters|ceiling/i,
);
rejects(
  "an oversized finding is rejected",
  (s) => { s.verdicts.software.signoff = false; s.verdicts.software.recommendations = ["y".repeat(5000)]; },
  /recommendation .* characters|ceiling/i,
);
rejects(
  "more findings than the cap is rejected",
  (s) => {
    s.verdicts.software.signoff = false;
    s.verdicts.software.recommendations = Array.from({ length: 65 }, (_, i) => `finding ${i}`);
  },
  /recommendations|transcript/i,
);

console.log("make-records: malformed input");
rejects(
  "a missing candidate address fails closed",
  (s) => { delete s.candidate.candidate_id; },
  /candidate_id/i,
);

{
  const dir = mkdtempSync(join(tmpdir(), "d2b-panel-empty-"));
  try {
    const r = run(dir);
    check("an empty round directory fails closed", r.code !== 0);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

if (failures) {
  console.error(`\ntest-make-records: ${failures} failure(s)`);
  process.exit(1);
}
console.log("\ntest-make-records: all cases passed");
