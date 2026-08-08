#!/usr/bin/env node
// Coverage for make-records.mjs, the helper that turns a selected roster of
// reviewer verdicts into the records `delivery wave panel-attest` consumes.
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
import { join, relative } from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import {
  adaptVerificationVerdict,
  createApprovalArtifact,
  sha256,
  stableStringify,
} from "../../.github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs";

const here = fileURLToPath(new URL(".", import.meta.url));
const root = join(here, "..", "..");
const script = join(root, ".github", "skills", "d2b-panel-round", "scripts", "make-records.mjs");

const ROLES = [
  "software", "test", "product", "docs", "security", "observability",
  "simplicity", "reliability", "agentic", "nixos", "networking", "kernel",
  "build",
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
      lifecycle_id: "spec001w1",
      base: "a".repeat(40),
      previous_tip: "b".repeat(40),
      tip: "c".repeat(40),
      delta_sha256: "d".repeat(64),
      full_sha256: "e".repeat(64),
      phase: "verification",
      selection_path: join(dir, "selection.json"),
    },
    candidate: {
      candidate_id: "cand-0001",
      content_id: "content-0001",
      snapshot_sha256: "f".repeat(64),
      program: "SPEC001",
      wave: "spec001w1",
      candidate_class: "code",
      changed_paths: ["src/lib.rs"],
      signals: ["rust"],
    },
    selection: {
      artifact_kind: "d2b-panel/lifecycle-selection",
      schema_version: 1,
      lifecycle_id: "spec001w1",
      phase: "verification",
      program: "SPEC001",
      wave: "spec001w1",
      candidate_id: "cand-0001",
      content_id: "content-0001",
      snapshot_sha256: "f".repeat(64),
      selection_table_version: 2,
      candidate_class: "code",
      classification_inputs: {
        changed_paths: ["src/lib.rs"],
        signals: ["rust"],
        candidate_class: "code",
        ambiguous: false,
        full_candidate: {
          changed_paths: ["src/lib.rs"],
          signals: ["rust"],
          candidate_class: "code",
          ambiguous: false,
        },
        fix_delta: {
          changed_paths: [],
          signals: [],
          candidate_class: "code",
          ambiguous: false,
        },
      },
      ambiguity_widened: false,
      profiles: Object.fromEntries(ROLES.map((role) => [role, role === "software" ? ["rust"] : []])),
      roster: ROLES,
    },
    ledger: {
      artifact_kind: "d2b-panel/issue-ledger",
      schema_version: 1,
      lifecycle_id: "spec001w1",
      selection_schema_version: 1,
      selection_table_version: 2,
      program: "SPEC001",
      wave: "spec001w1",
      candidate_id: "cand-0001",
      content_id: "content-0001",
      snapshot_sha256: "f".repeat(64),
      roster: ROLES,
      sources: [],
      issues: [],
      complete: true,
    },
    observed: Object.fromEntries(ROLES.map((r, i) => [r, {
      provider: "github-copilot",
      model: "gpt-5.6-sol",
      reasoning_effort: "xhigh",
      run_id: `run-${i}`,
      receipt_locator: `github-copilot://receipt/${i}`,
    }])),
    verdicts: Object.fromEntries(ROLES.map((r) => [r, {
      engineer: r,
      signoff: true,
      summary: "Verified.",
      recommendations: [],
      verified_issue_statuses: {},
      late_findings: [],
    }])),
  };

  const selectionBytes = stableStringify(state.selection);
  const ledgerBytes = stableStringify(state.ledger);
  state.responses = {
    artifact_kind: "d2b-panel/implementation-responses",
    schema_version: 1,
    selection_schema_version: 1,
    selection_table_version: 2,
    lifecycle_id: "spec001w1",
    program: "SPEC001",
    wave: "spec001w1",
    candidate_id: "cand-0001",
    content_id: "content-0001",
    snapshot_sha256: "f".repeat(64),
    roster: ROLES,
    responses: [],
  };
  state.verificationResults = {
    artifact_kind: "d2b-panel/verification",
    schema_version: 1,
    phase: "verification",
    lifecycle_id: "spec001w1",
    selection_sha256: sha256(selectionBytes),
    current_candidate: state.candidate,
    discovery_ledger_sha256: sha256(ledgerBytes),
    results: Object.fromEntries(ROLES.map((role) => [role, {
      seat: role,
      complete: true,
      signoff: true,
      summary: "Verified.",
      blocking_recommendations: [],
      recommendations: [],
      verified_issue_statuses: {},
      late_findings: [],
    }])),
  };
  const responseBytes = stableStringify(state.responses);
  const verificationResultsBytes = stableStringify(state.verificationResults);
  state.address.selection_sha256 = sha256(selectionBytes);
  state.approval = createApprovalArtifact({
    current_selection: state.selection,
    discovery_ledger: state.ledger,
    current_candidate: state.candidate,
    ledger_bytes: ledgerBytes,
    responses: state.responses,
    responses_bytes: responseBytes,
    verification_results: state.verificationResults,
    verification_results_bytes: verificationResultsBytes,
  });
  if (mutate) mutate(state);
  const finalSelectionBytes = stableStringify(state.selection);
  const finalLedgerBytes = stableStringify(state.ledger);
  writeFileSync(join(dir, "address.json"), stableStringify(state.address));
  writeFileSync(join(dir, "current-candidate.json"), stableStringify(state.candidate));
  writeFileSync(join(dir, "selection.json"), finalSelectionBytes);
  writeFileSync(join(dir, "ledger.json"), finalLedgerBytes);
  writeFileSync(join(dir, "approval.json"), stableStringify(state.approval));
  writeFileSync(join(dir, "responses.json"), stableStringify(state.responses));
  writeFileSync(join(dir, "verification-results.json"), stableStringify(state.verificationResults));
  writeFileSync(join(dir, "observed.json"), stableStringify(state.observed));
  for (const [role, v] of Object.entries(state.verdicts)) {
    writeFileSync(join(dir, "verdicts", `${role}.json`), stableStringify(v));
  }
  return dir;
}

function run(dir, selectionPath = join(dir, "selection.json")) {
  try {
    const stdout = execFileSync(
      "node",
      [
        script,
        dir,
        "--selection",
        selectionPath,
        "--ledger",
        join(dir, "ledger.json"),
        "--responses",
        join(dir, "responses.json"),
        "--verification-results",
        join(dir, "verification-results.json"),
        "--approval",
        join(dir, "approval.json"),
      ],
      { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] },
    );
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
    const relativeSelectionPath = `./${relative(process.cwd(), join(dir, "selection.json"))}`;
    const r = run(dir, relativeSelectionPath);
    check("a complete unanimous round is accepted", r.code === 0, `${r.err}`);
    const recordsDir = join(dir, "records");
    check("one record per seat is written", ROLES.every((x) => existsSync(join(recordsDir, `${x}.json`))));
    check("no temp file survives the write", !ROLES.some((x) => existsSync(join(recordsDir, `${x}.json.tmp`))));
    if (existsSync(join(recordsDir, "security.json"))) {
      const rec = JSON.parse(readFileSync(join(recordsDir, "security.json"), "utf8"));
      check("the record carries the observed effort", rec.reasoning_effort === "xhigh", JSON.stringify(rec.reasoning_effort));
      check("the record carries the candidate address", rec.candidate_id === "cand-0001");
      check("the record carries current panel format version", rec.panel_format_version === 1);
      check("the record digests the verdict", typeof rec.output_sha256 === "string" && rec.output_sha256.length === 64);
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

console.log("make-records: verdict tampering cannot bypass approval");
{
  const dir = buildRound((s) => {
    s.verdicts.agentic.signoff = false;
    s.verdicts.agentic.recommendations = [{
      severity: "critical",
      where: "packages/d2b-core/src/lib.rs:1",
      what: "the thing is wrong",
      why: "it breaks the contract",
      fix: "stop doing that",
    }];
  });
  try {
    const r = run(dir);
    check(
      "a verdict changed after approval is refused",
      r.code !== 0 &&
        /exact adapted verification-result bytes|approval artifact bytes|canonical inputs|recompute/.test(`${r.out}${r.err}`),
      `${r.out}${r.err}`,
    );
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

console.log("make-records: approval and publication preflight");
{
  const dir = buildRound();
  try {
    rmSync(join(dir, "approval.json"));
    const missingApproval = run(dir);
    check(
      "approval is required before current records",
      missingApproval.code !== 0 &&
        /approval artifact|usage/.test(`${missingApproval.out}${missingApproval.err}`),
    );
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}
{
  const dir = buildRound();
  try {
    const missingLedger = execFileSync(
      "node",
      [
        script,
        dir,
        "--selection",
        join(dir, "selection.json"),
        "--responses",
        join(dir, "responses.json"),
        "--verification-results",
        join(dir, "verification-results.json"),
        "--approval",
        join(dir, "approval.json"),
      ],
      { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] },
    );
    check("canonical ledger flag is required", false, missingLedger);
  } catch (cause) {
    check(
      "canonical ledger flag is required",
      cause.status === 2 && /--ledger <discovery-ledger\.json>/.test(
        `${cause.stdout ?? ""}${cause.stderr ?? ""}`,
      ),
    );
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}
{
  const dir = buildRound();
  try {
    const first = run(dir);
    const recordPath = join(dir, "records", "software.json");
    const before = readFileSync(recordPath, "utf8");
    writeFileSync(recordPath, `${before}tampered\n`);
    const conflict = run(dir);
    check(
      "conflicting record bytes are refused after complete preflight",
      first.code === 0 &&
        conflict.code !== 0 &&
        /conflicting generated record bytes/.test(`${conflict.out}${conflict.err}`) &&
        readFileSync(recordPath, "utf8") !== before,
      `first=${first.code} conflict=${conflict.code} output=${conflict.out}${conflict.err}`,
    );
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

rejects(
  "tampered implementation response bytes are refused",
  (s) => {
    s.responses.responses = [{
      issue_id: "unexpected",
      disposition: "Fixed",
      changed_surface: ["x"],
      justification: "tampered",
      evidence: "tampered",
    }];
  },
  /implementation responses|exact implementation response bytes|approval artifact bytes|canonical inputs/,
);
rejects(
  "tampered adapted verification bytes are refused",
  (s) => {
    s.verificationResults.results.software.summary = "tampered";
  },
  /adapted verification-result bytes|approval artifact bytes|canonical inputs/,
);
rejects(
  "tampered approval bytes are refused",
  (s) => {
    s.approval.approved = false;
  },
  /approval artifact|canonical inputs/,
);

console.log("make-records: legacy Gemini is rejected by current records");
{
  const dir = buildRound((s) => {
    for (const observed of Object.values(s.observed)) {
      observed.model = "gemini-3.1-pro-preview";
      observed.reasoning_effort = "high";
    }
  });
  try {
    const r = run(dir);
    check(
      "a current record round rejects the legacy Gemini binding",
      r.code !== 0 && /current records|policy accepts only/.test(`${r.out}${r.err}`),
      `${r.err}`,
    );
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
  (s) => {   s.observed.agentic.model = "claude-opus-5"; },
  /claude-opus-5|policy pins/i,
);
rejects(
  "legacy model and current effort cannot be mixed",
  (s) => {
    s.observed.agentic.model = "gemini-3.1-pro-preview";
    s.observed.agentic.reasoning_effort = "xhigh";
  },
  /legacy|compatibility pair|xhigh/i,
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
rejects(
  "a selection candidate mismatch fails closed",
  (s) => { s.selection.candidate_id = "other-candidate"; },
  /selection candidate mismatch|candidate_id/i,
);
rejects(
  "a selection schema mismatch fails closed",
  (s) => { s.selection.schema_version = 2; },
  /schema_version/i,
);
rejects(
  "a selection table version mismatch fails closed",
  (s) => { s.selection.selection_table_version = 1; },
  /selection table version|selection_table_version/i,
);
rejects(
  "a selection roster mismatch fails closed",
  (s) => { s.selection.roster = s.selection.roster.slice(0, -1); },
  /mandatory|roster/i,
);
rejects(
  "a record consumer rejects malformed nested classification",
  (s) => {
    s.selection.classification_inputs.full_candidate = {
      changed_paths: ["src/lib.rs"],
      signals: ["rust"],
      candidate_class: "code",
      ambiguous: false,
      unexpected: true,
    };
  },
  /unknown field|classification/i,
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
