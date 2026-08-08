#!/usr/bin/env node
// Join selected-roster verdicts to a candidate address and emit current
// schema-version-2 panel records.
//
//   node make-records.mjs <round-dir> --selection <selection.json>
//
// The selection artifact is the one roster authority shared by the lifecycle
// helper and delivery tooling. This helper does not retain a fixed current
// roster and never silently treats an absent seat as zero findings.

import { createHash } from "node:crypto";
import {
  existsSync,
  linkSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { join } from "node:path";
import {
  readSelection,
  validateSelectionCandidate,
  validateSelectionAgainstTable,
  validateLedger,
  validateApprovalArtifact,
  sha256,
  stableStringify,
} from "./panel-lifecycle.mjs";

const PROVIDER_POLICY = "github-copilot";
const MODEL_POLICY = "gpt-5.6-sol";
const EFFORT_POLICY = "xhigh";
const LEGACY_MODEL_POLICY = "gemini-3.1-pro-preview";
const LEGACY_EFFORT_POLICY = "high";
const ARTIFACT_KIND = "d2b-delivery/panel-receipt";
const SCHEMA_VERSION = 2;
const PANEL_FORMAT_VERSION = 1;
const MAX_RECOMMENDATIONS = 64;
// Reviewer-authored free text is the only unbounded input on the sealing path.
const MAX_SUMMARY_CHARS = 4000;
const MAX_RECOMMENDATION_CHARS = 4000;

const errors = [];
const fail = (message) => errors.push(message);

function usage() {
  return "usage: make-records.mjs <round-dir> --selection <selection.json> --approval <approval.json>";
}

const dir = process.argv[2];
const selectionIndex = process.argv.indexOf("--selection");
const approvalIndex = process.argv.indexOf("--approval");
const selectionPath = selectionIndex >= 0 ? process.argv[selectionIndex + 1] : undefined;
const approvalPath = approvalIndex >= 0 ? process.argv[approvalIndex + 1] : undefined;
if (
  !dir ||
  selectionIndex < 0 ||
  !selectionPath ||
  approvalIndex < 0 ||
  !approvalPath ||
  selectionPath.startsWith("--") ||
  approvalPath.startsWith("--")
) {
  console.error(usage());
  process.exit(2);
}

const readJson = (path, label) => {
  if (!existsSync(path)) {
    fail(`missing ${label} at ${path}`);
    return null;
  }
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch (cause) {
    fail(`invalid ${label} at ${path}: ${cause.message}`);
    return null;
  }
};

const address = readJson(join(dir, "address.json"), "round address");
const candidate = readJson(join(dir, "candidate.json"), "candidate address");
const observed = readJson(join(dir, "observed.json"), "observed binding table");
const approval = readJson(approvalPath, "approval artifact");
const discoveryLedgerPath = existsSync(join(dir, "discovery-ledger.json"))
  ? join(dir, "discovery-ledger.json")
  : join(dir, "ledger.json");
let discoveryLedgerBytes = "";
const discoveryLedger = readJson(discoveryLedgerPath, "immutable discovery ledger");
try {
  discoveryLedgerBytes = readFileSync(discoveryLedgerPath, "utf8");
} catch (cause) {
  fail(`missing immutable discovery ledger at ${discoveryLedgerPath}: ${cause.message}`);
}
let selection = null;
try {
  selection = readSelection(selectionPath);
} catch (cause) {
  fail(`invalid lifecycle selection at ${selectionPath}: ${cause.message}`);
}

if (errors.length) {
  for (const message of errors) console.error(`error: ${message}`);
  process.exit(1);
}
try {
  validateLedger(discoveryLedger);
} catch (cause) {
  fail(`invalid immutable discovery ledger: ${cause.message}`);
}

let selectionBytes;
try {
  selectionBytes = readFileSync(selectionPath, "utf8");
} catch (cause) {
  fail(`cannot read lifecycle selection bytes at ${selectionPath}: ${cause.message}`);
}
try {
  readFileSync(approvalPath, "utf8");
} catch (cause) {
  fail(`cannot read approval bytes at ${approvalPath}: ${cause.message}`);
}
if (errors.length) {
  for (const message of errors) console.error(`error: ${message}`);
  process.exit(1);
}
if (selection?.phase !== "verification") {
  fail("current records require a verification-phase lifecycle selection");
}
try {
  validateSelectionAgainstTable(selection);
} catch (cause) {
  fail(`selection does not satisfy the authoritative selection table: ${cause.message}`);
}
if (selection && address?.selection_sha256 !== sha256(selectionBytes)) {
  fail("address.json selection_sha256 does not match the recorded selection bytes");
}
try {
  validateApprovalArtifact(approval, {
    selection,
    ledgerBytes: discoveryLedgerBytes,
  });
} catch (cause) {
  fail(`invalid approval artifact: ${cause.message}`);
}
if (approval && approval.selection_sha256 !== sha256(selectionBytes)) {
  fail("approval artifact selection_sha256 does not match the recorded selection bytes");
}
if (approval && !approval.approved) {
  fail("approval artifact is not approved; current records require merge-ready verification");
}
if (approval && approval.discovery_ledger_sha256 !==
    sha256(discoveryLedgerBytes)) {
  fail("approval artifact is not bound to the immutable discovery ledger bytes");
}

if (!candidate || typeof candidate !== "object" || Array.isArray(candidate)) {
  fail("candidate.json must be an object");
} else {
  for (const key of ["candidate_id", "content_id", "snapshot_sha256"]) {
    if (typeof candidate[key] !== "string" || !candidate[key]) {
      fail(`candidate.json is missing ${key}`);
    }
  }
}
if (
  address?.lifecycle_id !== selection.lifecycle_id ||
  address?.selection_path !== selectionPath
) {
  fail("address.json must bind the exact lifecycle and selection path used for records");
}
if (
  approval?.current_candidate &&
  candidate &&
  ["candidate_id", "content_id", "snapshot_sha256"].some(
    (key) => approval.current_candidate[key] !== candidate[key],
  )
) {
  fail("approval artifact current candidate disagrees with staged candidate.json");
}
try {
  validateSelectionCandidate(selection, candidate);
} catch (cause) {
  fail(`selection candidate mismatch: ${cause.message}`);
}
if (!address || typeof address.lifecycle_id !== "string" || !address.lifecycle_id) {
  fail("address.json lifecycle_id is required for selected-roster records");
} else if (address.lifecycle_id !== selection.lifecycle_id) {
  fail(
    `address.json lifecycle_id "${address.lifecycle_id}" disagrees with selection "${selection.lifecycle_id}"`,
  );
}

const verdictDir = join(dir, "verdicts");
const observedTable = observed && typeof observed === "object" && !Array.isArray(observed)
  ? observed
  : {};
if (!observed || typeof observed !== "object" || Array.isArray(observed)) {
  fail("observed.json must be an object keyed by selected seat");
}
const observedKeys = observedTable
  ? Object.keys(observedTable)
  : [];
const present = existsSync(verdictDir)
  ? readdirSync(verdictDir)
      .filter((file) => file.endsWith(".json"))
      .map((file) => file.slice(0, -5))
  : [];
const roster = selection.roster;
const expected = new Set(roster);
for (const seat of present) {
  if (!expected.has(seat)) {
    fail(`verdict for unknown seat "${seat}"; selection roster is [${roster.join(", ")}]`);
  }
}
for (const seat of observedKeys) {
  if (!expected.has(seat)) {
    fail(`observed.json has unknown seat "${seat}"; selection roster is [${roster.join(", ")}]`);
  }
}
for (const seat of roster) {
  if (!present.includes(seat)) {
    fail(`no verdict for selected seat "${seat}"; every selected seat is required`);
  }
  if (!observedKeys.includes(seat)) {
    fail(`observed.json has no entry for selected seat "${seat}"`);
  }
}

const seenRunIds = new Set();
const seenReceipts = new Set();
const records = [];

// A string passes through untouched. The structured shape is rendered in a
// fixed field order, so the same finding has the same output digest. Anything
// else falls back to JSON rather than being dropped.
function renderRecommendation(recommendation) {
  if (typeof recommendation === "string") return recommendation;
  if (
    recommendation &&
    typeof recommendation === "object" &&
    !Array.isArray(recommendation)
  ) {
    const {
      severity,
      where,
      what,
      why,
      fix,
    } = recommendation;
    const fields = [severity, where, what, why, fix];
    if (fields.every((field) => typeof field === "string" && field.length > 0)) {
      return `[${severity}] ${where}: ${what} Why: ${why} Fix: ${fix}`;
    }
  }
  return JSON.stringify(recommendation);
}

for (const role of roster) {
  if (!present.includes(role)) continue;
  const verdict = readJson(
    join(verdictDir, `${role}.json`),
    `verdict for ${role}`,
  );
  if (!verdict) continue;
  if (verdict.engineer !== role) {
    fail(
      `verdict ${role}.json declares engineer "${verdict.engineer}"; file name and seat must agree`,
    );
  }
  if (!Array.isArray(verdict.recommendations)) {
    fail(`verdict ${role}: recommendations must be an array`);
    continue;
  }
  if (typeof verdict.signoff !== "boolean") {
    fail(`verdict ${role}: signoff must be a boolean`);
    continue;
  }
  if (verdict.signoff !== (verdict.recommendations.length === 0)) {
    fail(
      `verdict ${role}: signoff is ${verdict.signoff} with ` +
      `${verdict.recommendations.length} recommendations; signoff is true if and only if recommendations is empty`,
    );
  }
  if (verdict.recommendations.length > MAX_RECOMMENDATIONS) {
    fail(
      `verdict ${role}: more than ${MAX_RECOMMENDATIONS} recommendations; a record is a verdict, not a transcript`,
    );
  }
  if (typeof verdict.summary !== "string" || !verdict.summary.trim()) {
    fail(`verdict ${role}: summary is required`);
  }
  if (
    typeof verdict.summary === "string" &&
    verdict.summary.length > MAX_SUMMARY_CHARS
  ) {
    fail(
      `verdict ${role}: summary is ${verdict.summary.length} characters, over the ` +
      `${MAX_SUMMARY_CHARS} ceiling`,
    );
  }
  const recommendations = verdict.recommendations.map(renderRecommendation);
  for (const [index, recommendation] of recommendations.entries()) {
    if (recommendation.length > MAX_RECOMMENDATION_CHARS) {
      fail(
        `verdict ${role}: recommendation ${index} is ${recommendation.length} characters, over the ` +
        `${MAX_RECOMMENDATION_CHARS} ceiling`,
      );
    }
  }

  const observedBinding = observedTable[role];
  if (!observedBinding) continue;
  if (
    typeof observedBinding !== "object" ||
    Array.isArray(observedBinding)
  ) {
    fail(`observed.json ${role} must be an object`);
    continue;
  }
  for (const key of ["provider", "model", "reasoning_effort", "run_id", "receipt_locator"]) {
    if (
      typeof observedBinding[key] !== "string" ||
      observedBinding[key].length === 0
    ) {
      fail(`observed.json ${role}: ${key} is required`);
    }
  }
  const currentBinding =
    observedBinding.model === MODEL_POLICY &&
    observedBinding.reasoning_effort === EFFORT_POLICY;
  if (!currentBinding) {
    fail(
      `observed.json ${role}: lane ran on "${observedBinding.model}" at effort ` +
      `"${observedBinding.reasoning_effort}", but policy accepts only ` +
      `"${MODEL_POLICY}"/"${EFFORT_POLICY}" for current records`,
    );
  }
  const provider = observedBinding.provider;
  if (provider !== PROVIDER_POLICY) {
    fail(
      `observed.json ${role}: provider "${provider}" but policy pins "${PROVIDER_POLICY}"`,
    );
  }
  if (seenRunIds.has(observedBinding.run_id)) {
    fail(`run_id "${observedBinding.run_id}" is used by more than one seat`);
  }
  if (seenReceipts.has(observedBinding.receipt_locator)) {
    fail(
      `receipt_locator "${observedBinding.receipt_locator}" is used by more than one seat`,
    );
  }
  seenRunIds.add(observedBinding.run_id);
  seenReceipts.add(observedBinding.receipt_locator);
  if (!observedBinding.receipt_locator.startsWith(`${provider}://`)) {
    fail(
      `observed.json ${role}: receipt_locator must start with "${provider}://"`,
    );
  }

  const verdictBody = stableStringify({
    engineer: role,
    signoff: verdict.signoff,
    summary: verdict.summary,
    recommendations,
  });
  records.push({
    artifact_kind: ARTIFACT_KIND,
    schema_version: SCHEMA_VERSION,
    panel_format_version: PANEL_FORMAT_VERSION,
    role,
    candidate_id: candidate.candidate_id,
    content_id: candidate.content_id,
    snapshot_sha256: candidate.snapshot_sha256,
    model_version: observedBinding.model,
    provider,
    reasoning_effort: observedBinding.reasoning_effort,
    run_id: observedBinding.run_id,
    receipt_locator: observedBinding.receipt_locator,
    output_sha256: createHash("sha256").update(verdictBody).digest("hex"),
    signoff: verdict.signoff,
    recommendations,
  });
}

if (errors.length) {
  for (const message of errors) console.error(`error: ${message}`);
  process.exit(1);
}

const outDir = join(dir, "records");
const expectedFiles = new Set(records.map((record) => `${record.role}.json`));
const existingFiles = existsSync(outDir)
  ? readdirSync(outDir)
  : [];
for (const file of existingFiles) {
  if (!expectedFiles.has(file)) {
    fail(`unexpected pre-existing record ${join(outDir, file)}; refusing a mixed output set`);
  }
}
const pendingWrites = records.map((record) => ({
  final: join(outDir, `${record.role}.json`),
  bytes: `${JSON.stringify(record, null, 2)}\n`,
}));
for (const { final, bytes } of pendingWrites) {
  if (existsSync(final) && readFileSync(final, "utf8") !== bytes) {
    fail(`conflicting generated record bytes at ${final}; refusing to overwrite`);
  }
}
if (errors.length) {
  for (const message of errors) console.error(`error: ${message}`);
  process.exit(1);
}

mkdirSync(outDir, { recursive: true });
let publicationCounter = 0;
for (const { final, bytes } of pendingWrites) {
  if (existsSync(final)) continue;
  const temporary = `${final}.${process.pid}.${publicationCounter += 1}.tmp`;
  try {
    writeFileSync(temporary, bytes, { encoding: "utf8", flag: "wx" });
    try {
      linkSync(temporary, final);
    } catch (cause) {
      if (cause.code !== "EEXIST") throw cause;
      const actual = readFileSync(final, "utf8");
      if (actual !== bytes) {
        fail(`conflicting generated record bytes at ${final}; refusing to overwrite`);
      }
    }
  } finally {
    try {
      unlinkSync(temporary);
    } catch {
      // The temporary file may not have been created.
    }
  }
}

if (errors.length) {
  for (const message of errors) console.error(`error: ${message}`);
  process.exit(1);
}

const findings = records.filter((record) => !record.signoff);
console.log(`wrote ${records.length} records to ${outDir}`);
console.log(`selection ${selectionPath}`);
if (findings.length === 0) {
  console.log(`verdict: unanimous ${records.length}/${roster.length}, round passes`);
  process.exit(0);
}
console.log(
  `verdict: ${records.length - findings.length}/${roster.length}, round does NOT pass`,
);
for (const record of findings) {
  console.log(
    `  ${record.role}: ${record.recommendations.length} finding(s)`,
  );
}
console.log("\nLand fixes scoped to these findings only, revalidate, and run scoped verification.");
process.exit(3);
