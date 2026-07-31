#!/usr/bin/env node
// Join panel verdicts to a candidate address and emit attestable records.
//
//   node make-records.mjs <round-dir>
//
// Reads from <round-dir>:
//   address.json    written by stage-diffs.sh
//   candidate.json  {candidate_id, content_id, snapshot_sha256, program, wave}
//   observed.json   {"<seat>": {model, reasoning_effort, run_id, receipt_locator}}
//   verdicts/<seat>.json
//
// Writes <round-dir>/records/<seat>.json, ready for `delivery wave panel-attest`.
//
// This script fails closed. It never substitutes the policy model or effort
// for an unreported observed value, because a lane dispatched without an
// explicit reasoning effort silently runs at the model default while the
// record would attest the policy level. That is the one failure mode on this
// path that produces a plausible-looking artifact rather than an error.

import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, readdirSync, renameSync, writeFileSync } from "node:fs";
import { join } from "node:path";

// Mirrors packages/xtask/src/delivery/model.rs.
const ROLES = [
  "software", "test", "nixos", "networking", "security",
  "rust", "product", "docs", "observability", "kernel",
];
const PROVIDER_POLICY = "github-copilot";
const MODEL_POLICY = "gemini-3.1-pro-preview";
const EFFORT_POLICY = "high";
const ARTIFACT_KIND = "d2b-delivery/panel-receipt";
const SCHEMA_VERSION = 2;
const MAX_RECOMMENDATIONS = 64;
// Reviewer-authored free text is the only unbounded input on the sealing path.
// `panel.rs` caps the array; these cap each element and the summary.
const MAX_SUMMARY_CHARS = 4000;
const MAX_RECOMMENDATION_CHARS = 4000;

const errors = [];
const fail = (m) => errors.push(m);

const dir = process.argv[2];
if (!dir) {
  console.error("usage: make-records.mjs <round-dir>");
  process.exit(2);
}

const readJson = (path, label) => {
  if (!existsSync(path)) {
    fail(`missing ${label} at ${path}`);
    return null;
  }
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch (e) {
    fail(`invalid ${label} at ${path}: ${e.message}`);
    return null;
  }
};

const address = readJson(join(dir, "address.json"), "round address");
const candidate = readJson(join(dir, "candidate.json"), "candidate address");
const observed = readJson(join(dir, "observed.json"), "observed binding table");

if (errors.length) {
  for (const e of errors) console.error(`error: ${e}`);
  console.error(
    "\nobserved.json must record what each lane actually ran at. It is not\n" +
    "optional and it is not defaulted: a record that attests an effort the\n" +
    "lane did not use is a false attestation on the binding gate.",
  );
  process.exit(1);
}

for (const k of ["candidate_id", "content_id", "snapshot_sha256"]) {
  if (typeof candidate[k] !== "string" || !candidate[k]) {
    fail(`candidate.json is missing ${k}`);
  }
}

// Verdicts.
const verdictDir = join(dir, "verdicts");
const present = existsSync(verdictDir)
  ? readdirSync(verdictDir).filter((f) => f.endsWith(".json")).map((f) => f.slice(0, -5))
  : [];

for (const seat of present) {
  if (!ROLES.includes(seat)) fail(`verdict for unknown seat "${seat}"; roster is closed`);
}
for (const role of ROLES) {
  if (!present.includes(role)) fail(`no verdict for seat "${role}"; all ten are required`);
}

const seenRunIds = new Set();
const seenReceipts = new Set();
const records = [];

for (const role of ROLES) {
  if (!present.includes(role)) continue;
  const v = readJson(join(verdictDir, `${role}.json`), `verdict for ${role}`);
  if (!v) continue;

  if (v.engineer !== role) {
    fail(`verdict ${role}.json declares engineer "${v.engineer}"; file name and seat must agree`);
  }
  if (!Array.isArray(v.recommendations)) {
    fail(`verdict ${role}: recommendations must be an array`);
    continue;
  }
  if (typeof v.signoff !== "boolean") {
    fail(`verdict ${role}: signoff must be a boolean`);
    continue;
  }
  if (v.signoff !== (v.recommendations.length === 0)) {
    fail(
      `verdict ${role}: signoff is ${v.signoff} with ${v.recommendations.length} ` +
      `recommendations. signoff is true if and only if recommendations is empty; ` +
      `there is no partial pass.`,
    );
  }
  if (v.recommendations.length > MAX_RECOMMENDATIONS) {
    fail(`verdict ${role}: more than ${MAX_RECOMMENDATIONS} recommendations; a record is a verdict, not a transcript`);
  }
  if (typeof v.summary !== "string" || !v.summary.trim()) {
    fail(`verdict ${role}: summary is required`);
  }
  // A record is a bounded structured artifact, not a place to spill a
  // transcript. Capping the reviewer-authored strings keeps a verbose lane
  // from producing an unbounded payload on the sealing path.
  if (typeof v.summary === "string" && v.summary.length > MAX_SUMMARY_CHARS) {
    fail(
      `verdict ${role}: summary is ${v.summary.length} characters, over the ` +
      `${MAX_SUMMARY_CHARS} ceiling. State the posture and the findings; the diff is ` +
      `the evidence and does not belong in the record.`,
    );
  }
  for (const [i, rec] of v.recommendations.entries()) {
    const text = typeof rec === "string" ? rec : JSON.stringify(rec);
    if (text.length > MAX_RECOMMENDATION_CHARS) {
      fail(
        `verdict ${role}: recommendation ${i} is ${text.length} characters, over the ` +
        `${MAX_RECOMMENDATION_CHARS} ceiling. A finding names the defect, where it is, ` +
        `and the fix; it does not quote the file.`,
      );
    }
  }

  const o = observed[role];
  if (!o) {
    fail(`observed.json has no entry for seat "${role}"`);
    continue;
  }
  for (const k of ["model", "reasoning_effort", "run_id", "receipt_locator"]) {
    if (typeof o[k] !== "string" || !o[k]) fail(`observed.json ${role}: ${k} is required`);
  }
  if (o.model !== MODEL_POLICY) {
    fail(
      `observed.json ${role}: lane ran on "${o.model}" but policy pins "${MODEL_POLICY}". ` +
      `Re-dispatch that seat; the record cannot be written.`,
    );
  }
  if (o.reasoning_effort !== EFFORT_POLICY) {
    fail(
      `observed.json ${role}: lane ran at effort "${o.reasoning_effort}" but policy pins ` +
      `"${EFFORT_POLICY}". This is the silent-downgrade case: the dispatch almost certainly ` +
      `omitted reasoning_effort. Re-dispatch that seat with it set explicitly.`,
    );
  }
  const provider = o.provider ?? PROVIDER_POLICY;
  if (provider !== PROVIDER_POLICY) {
    fail(`observed.json ${role}: provider "${provider}" but policy pins "${PROVIDER_POLICY}"`);
  }
  if (o.run_id && seenRunIds.has(o.run_id)) {
    fail(`run_id "${o.run_id}" is used by more than one seat; each seat's provenance must be distinct`);
  }
  if (o.receipt_locator && seenReceipts.has(o.receipt_locator)) {
    fail(`receipt_locator "${o.receipt_locator}" is used by more than one seat`);
  }
  if (o.run_id) seenRunIds.add(o.run_id);
  if (o.receipt_locator) {
    seenReceipts.add(o.receipt_locator);
    if (!o.receipt_locator.startsWith(`${provider}://`)) {
      fail(`observed.json ${role}: receipt_locator must start with "${provider}://"`);
    }
  }

  const verdictBody = JSON.stringify({
    engineer: role,
    signoff: v.signoff,
    summary: v.summary,
    recommendations: v.recommendations,
  });

  records.push({
    artifact_kind: ARTIFACT_KIND,
    schema_version: SCHEMA_VERSION,
    role,
    candidate_id: candidate.candidate_id,
    content_id: candidate.content_id,
    snapshot_sha256: candidate.snapshot_sha256,
    model_version: o.model,
    provider,
    reasoning_effort: o.reasoning_effort,
    run_id: o.run_id,
    receipt_locator: o.receipt_locator,
    output_sha256: createHash("sha256").update(verdictBody).digest("hex"),
    signoff: v.signoff,
    recommendations: v.recommendations,
  });
}

if (errors.length) {
  for (const e of errors) console.error(`error: ${e}`);
  process.exit(1);
}

const outDir = join(dir, "records");
mkdirSync(outDir, { recursive: true });
// Write-then-rename. A record truncated by a signal or a full disk would
// otherwise sit at its final path and be consumed as a complete attestation.
for (const r of records) {
  const final = join(outDir, `${r.role}.json`);
  const tmp = `${final}.tmp`;
  writeFileSync(tmp, `${JSON.stringify(r, null, 2)}\n`);
  renameSync(tmp, final);
}

const findings = records.filter((r) => !r.signoff);
console.log(`wrote ${records.length} records to ${outDir}`);
console.log(`round tip ${address.tip}`);
if (findings.length === 0) {
  console.log("verdict: unanimous 10/10, round passes");
  process.exit(0);
}
console.log(`verdict: ${10 - findings.length}/10, round does NOT pass`);
for (const r of findings) {
  console.log(`  ${r.role}: ${r.recommendations.length} finding(s)`);
}
console.log("\nLand fixes scoped to these findings only, revalidate, and run another round.");
process.exit(3);
