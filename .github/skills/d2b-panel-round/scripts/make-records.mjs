#!/usr/bin/env node
// Join selected-roster verdicts to a candidate address and emit current
// schema-version-2 panel records.
//
//   node make-records.mjs <round-dir> --selection <selection.json> \
//     --ledger <discovery-ledger.json> --responses <responses.json> \
//     --verification-results <verification-results.json> --approval <approval.json>
//
// The selection artifact is the one roster authority shared by the lifecycle
// helper and delivery tooling. This helper does not retain a fixed current
// roster and never silently treats an absent seat as zero findings.

import { createHash } from "node:crypto";
import {
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import {
  adaptVerificationVerdict,
  createApprovalArtifact,
  validateSelection,
  validateSelectionCandidate,
  validateSelectionAgainstTable,
  validateLedger,
  validateApprovalArtifact,
  validateResponses,
  validateVerificationResultArtifact,
  sha256,
  stableStringify,
} from "./panel-lifecycle.mjs";

const PROVIDER_POLICY = "github-copilot";
const MODEL_POLICY = "gpt-5.6-sol";
const EFFORT_POLICY = "xhigh";
const LEGACY_MODEL_POLICY = "gpt-5.6-sol";
const LEGACY_EFFORT_POLICY = "xhigh";
const ARTIFACT_KIND = "d2b-delivery/panel-receipt";
const SCHEMA_VERSION = 2;
const PANEL_FORMAT_VERSION = 1;
const MAX_RECOMMENDATIONS = 64;
// Reviewer-authored free text is bounded before it reaches a generated record.
const MAX_SUMMARY_CHARS = 4000;
const MAX_RECOMMENDATION_CHARS = 4000;
const MAX_COMPLETION_MARKER_BYTES = 256 * 1024;
const MAX_STAGED_ARTIFACT_BYTES = 64 * 1024 * 1024;
const MAX_AGENT_DEFINITION_BYTES = 1024 * 1024;
const MAX_POST_STAGE_JSON_BYTES = 16 * 1024 * 1024;
const MAX_VERDICT_BYTES = 512 * 1024;
const MAX_RECORD_BYTES = 512 * 1024;
const MAX_ARTIFACT_PATH_CHARS = 1024;

const errors = [];
const fail = (message) => errors.push(message);

function usage() {
  return (
    "usage: make-records.mjs <round-dir> --selection <selection.json> " +
    "--ledger <discovery-ledger.json> --responses <responses.json> " +
    "--verification-results <verification-results.json> --approval <approval.json>"
  );
}

const REQUIRED_FLAGS = new Map([
  ["--selection", "selectionPath"],
  ["--ledger", "ledgerPath"],
  ["--responses", "responsesPath"],
  ["--verification-results", "verificationResultsPath"],
  ["--approval", "approvalPath"],
]);

function parseArguments(argv) {
  let roundDir;
  const values = {};
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (REQUIRED_FLAGS.has(argument)) {
      const key = REQUIRED_FLAGS.get(argument);
      if (Object.hasOwn(values, key)) {
        throw new Error(`option ${argument} may be supplied only once`);
      }
      const value = argv[index + 1];
      if (
        value === undefined ||
        value.length === 0 ||
        value.startsWith("-")
      ) {
        throw new Error(`option ${argument} requires one value`);
      }
      values[key] = value;
      index += 1;
      continue;
    }
    if (argument.startsWith("-")) {
      throw new Error(`unknown option "${argument}"`);
    }
    if (roundDir !== undefined) {
      throw new Error(`unexpected positional argument "${argument}"`);
    }
    if (argument.length === 0) {
      throw new Error("round directory positional must not be empty");
    }
    roundDir = argument;
  }
  if (roundDir === undefined) {
    throw new Error("exactly one round directory positional is required");
  }
  for (const [flag, key] of REQUIRED_FLAGS) {
    if (!Object.hasOwn(values, key)) {
      throw new Error(`missing required option ${flag}`);
    }
  }
  return { dir: roundDir, ...values };
}

let parsedArguments;
try {
  parsedArguments = parseArguments(process.argv.slice(2));
} catch (cause) {
  console.error(usage());
  console.error(`error: ${cause.message}`);
  process.exit(2);
}

const {
  dir,
  selectionPath,
  ledgerPath,
  responsesPath,
  verificationResultsPath,
  approvalPath,
} = parsedArguments;

function readLimitedBytes(path, label, maxBytes) {
  const stat = statSync(path);
  if (!stat.isFile()) {
    throw new Error(`${label} is not a regular file`);
  }
  if (stat.size > maxBytes) {
    throw new Error(`${label} exceeds ${maxBytes} bytes`);
  }
  const bytes = readFileSync(path);
  if (bytes.length > maxBytes) {
    throw new Error(`${label} exceeds ${maxBytes} bytes`);
  }
  return bytes;
}

function parseJsonBytes(bytes, path, label) {
  try {
    const text = bytes.toString("utf8");
    return {
      bytes,
      text,
      value: JSON.parse(text),
    };
  } catch (cause) {
    throw new Error(`invalid ${label} at ${path}: ${cause.message}`);
  }
}

function readPostStageJson(path, label, maxBytes = MAX_POST_STAGE_JSON_BYTES) {
  let bytes;
  try {
    bytes = readLimitedBytes(path, label, maxBytes);
  } catch (cause) {
    if (cause.code === "ENOENT") {
      fail(`missing ${label} at ${path}`);
    } else {
      fail(`cannot read ${label} bytes at ${path}: ${cause.message}`);
    }
    return { bytes: null, text: "", value: null };
  }
  try {
    return parseJsonBytes(bytes, path, label);
  } catch (cause) {
    fail(cause.message);
    return { bytes, text: bytes.toString("utf8"), value: null };
  }
}

function stagedArtifactLimit(relativePath) {
  return relativePath.startsWith("agent-definitions/")
    ? MAX_AGENT_DEFINITION_BYTES
    : MAX_STAGED_ARTIFACT_BYTES;
}

function readCompletionBoundArtifacts(roundDir) {
  const markerPath = join(roundDir, ".complete");
  const markerBytes = readLimitedBytes(
    markerPath,
    "completion marker",
    MAX_COMPLETION_MARKER_BYTES,
  );
  const marker = JSON.parse(markerBytes.toString("utf8"));
  const expectedKeys = [
    "artifact_bytes",
    "artifact_kind",
    "artifact_sha256",
    "base",
    "complete",
    "delta_sha256",
    "full_sha256",
    "lifecycle_id",
    "phase",
    "previous_tip",
    "round",
    "schema_version",
    "selection_sha256",
    "tip",
  ].sort();
  if (
    marker.artifact_kind !== "d2b-panel/stage-completion" ||
    marker.complete !== true ||
    marker.phase !== "verification" ||
    Object.keys(marker).sort().join("\0") !== expectedKeys.join("\0")
  ) {
    throw new Error(
      "completion marker is not the current canonical verification packet",
    );
  }
  if (marker.schema_version !== 4) {
    throw new Error(
      `completion marker schema_version ${JSON.stringify(marker.schema_version)} ` +
      "is predecessor-only; current records require schema-version 4",
    );
  }
  const digests = marker.artifact_sha256;
  const sizes = marker.artifact_bytes;
  if (
    !digests ||
    Array.isArray(digests) ||
    typeof digests !== "object" ||
    !sizes ||
    Array.isArray(sizes) ||
    typeof sizes !== "object" ||
    Object.keys(digests).sort().join("\0") !== Object.keys(sizes).sort().join("\0")
  ) {
    throw new Error("completion marker artifact size and digest maps disagree");
  }
  const selectionBytes = readLimitedBytes(
    join(roundDir, "selection.json"),
    "completion-bound selection.json",
    MAX_STAGED_ARTIFACT_BYTES,
  );
  const selectionDigest = digests["selection.json"];
  const selectionSize = sizes["selection.json"];
  if (
    typeof selectionDigest !== "string" ||
    !/^[0-9a-f]{64}$/u.test(selectionDigest) ||
    !Number.isSafeInteger(selectionSize) ||
    selectionSize < 0 ||
    selectionSize > MAX_STAGED_ARTIFACT_BYTES ||
    selectionBytes.length !== selectionSize ||
    createHash("sha256").update(selectionBytes).digest("hex") !==
      selectionDigest
  ) {
    throw new Error(
      "completion-bound artifact selection.json has a different size or digest",
    );
  }
  let selection;
  try {
    selection = JSON.parse(selectionBytes.toString("utf8"));
  } catch (cause) {
    throw new Error(`completion-bound selection.json is not valid JSON: ${cause.message}`);
  }
  if (
    !selection ||
    typeof selection !== "object" ||
    Array.isArray(selection) ||
    !Array.isArray(selection.roster) ||
    selection.roster.length === 0 ||
    selection.roster.some(
      (seat) =>
        typeof seat !== "string" ||
        seat.length === 0 ||
        seat.includes("/") ||
        seat.includes("\\") ||
        seat.includes("\0"),
    ) ||
    new Set(selection.roster).size !== selection.roster.length
  ) {
    throw new Error(
      "completion-bound selection.json must declare a unique selected roster",
    );
  }
  const expectedNames = [
    "address.json",
    "commits.txt",
    "current-candidate.json",
    "delta.diff",
    "dispatch-binding.json",
    "dispatch-prompt.txt",
    "evidence.md",
    "full.diff",
    "review-request.md",
    "selection.json",
    "discovery-ledger.json",
    "responses.json",
    "self-verification.json",
    ...selection.roster.flatMap((seat) => [
      `agent-definitions/panel-${seat}.agent.md`,
      `reviewer-notes/${seat}.md`,
      `verification/${seat}.json`,
    ]),
  ].sort();
  const expectedNamesWithHandoff = [...expectedNames, "handoff.json"].sort();
  const actualNames = Object.keys(digests).sort();
  const matchesExpected = [expectedNames, expectedNamesWithHandoff].some(
    (expected) =>
      actualNames.length === expected.length &&
      actualNames.every((name, index) => name === expected[index]),
  );
  if (!matchesExpected) {
    const expectedUnion = expectedNamesWithHandoff;
    const missing = expectedNames.filter((name) => !actualNames.includes(name));
    const extra = actualNames.filter((name) => !expectedUnion.includes(name));
    throw new Error(
      "completion marker schema_version 4 requires the exact current " +
      "verification artifact set for the selected roster; " +
      `missing [${missing.join(", ")}], extra [${extra.join(", ")}]`,
    );
  }
  const artifacts = new Map();
  for (const relativePath of Object.keys(digests).sort()) {
    if (
      relativePath.length === 0 ||
      relativePath.length > MAX_ARTIFACT_PATH_CHARS ||
      relativePath.startsWith("/") ||
      relativePath.includes("\\") ||
      relativePath.split("/").some((component) => component === "" || component === "." || component === "..") ||
      !/^[0-9a-f]{64}$/u.test(digests[relativePath]) ||
      !Number.isSafeInteger(sizes[relativePath]) ||
      sizes[relativePath] < 0 ||
      sizes[relativePath] > stagedArtifactLimit(relativePath)
    ) {
      throw new Error(`completion marker has an invalid artifact binding for ${relativePath}`);
    }
    const path = resolve(roundDir, relativePath);
    if (path !== roundDir && !path.startsWith(`${roundDir}/`)) {
      throw new Error(`completion marker artifact escapes the round directory: ${relativePath}`);
    }
    const bytes = readLimitedBytes(
      path,
      `completion-bound artifact ${relativePath}`,
      stagedArtifactLimit(relativePath),
    );
    const digest = createHash("sha256").update(bytes).digest("hex");
    if (digest !== digests[relativePath] || bytes.length !== sizes[relativePath]) {
      throw new Error(
        `completion-bound artifact ${relativePath} has a different size or digest`,
      );
    }
    artifacts.set(relativePath, bytes);
  }
  return { marker, artifacts };
}

function canonicalRoundPath(roundDir, supplied, name) {
  const expected = join(roundDir, name);
  if (typeof supplied !== "string" || resolve(supplied) !== expected) {
    throw new Error(`${name} must use the canonical round-local path ${expected}`);
  }
  return expected;
}

const DISPATCH_BINDING_KEYS = [
  "agent_type",
  "model",
  "reasoning_effort",
  "context_tier",
  "communication",
];
const OBSERVED_BINDING_KEYS = [
  "provider",
  "model",
  "reasoning_effort",
  "context_tier",
  "communication",
  "agent_type",
  "agent_definition_sha256",
  "run_id",
  "receipt_locator",
];

function validateDispatchBinding(value, selection) {
  if (
    !value ||
    typeof value !== "object" ||
    Array.isArray(value) ||
    Object.keys(value).sort().join("\0") !==
      ["artifact_kind", "bindings", "lifecycle_id", "phase", "roster", "schema_version"]
        .sort()
        .join("\0")
  ) {
    throw new Error("dispatch-binding.json has an unexpected shape");
  }
  if (
    value.artifact_kind !== "d2b-panel/dispatch-binding" ||
    value.schema_version !== 1 ||
    value.lifecycle_id !== selection.lifecycle_id ||
    value.phase !== selection.phase
  ) {
    throw new Error(
      "dispatch-binding.json does not bind the current lifecycle and phase",
    );
  }
  if (
    !Array.isArray(value.roster) ||
    value.roster.join("\0") !== selection.roster.join("\0")
  ) {
    throw new Error(
      "dispatch-binding.json roster does not exactly match the lifecycle selection",
    );
  }
  if (
    !value.bindings ||
    typeof value.bindings !== "object" ||
    Array.isArray(value.bindings) ||
    Object.keys(value.bindings).sort().join("\0") !==
      [...selection.roster].sort().join("\0")
  ) {
    throw new Error(
      "dispatch-binding.json bindings do not exactly match the selected roster",
    );
  }
  for (const seat of selection.roster) {
    const binding = value.bindings[seat];
    if (
      !binding ||
      typeof binding !== "object" ||
      Array.isArray(binding) ||
      Object.keys(binding).sort().join("\0") !==
        DISPATCH_BINDING_KEYS.slice().sort().join("\0")
    ) {
      throw new Error(`dispatch-binding.json binding for ${seat} has an unexpected shape`);
    }
    for (const key of DISPATCH_BINDING_KEYS) {
      if (typeof binding[key] !== "string" || binding[key].trim() === "") {
        throw new Error(`dispatch-binding.json binding for ${seat} has no ${key}`);
      }
    }
    if (binding.agent_type !== `panel-${seat}`) {
      throw new Error(
        `dispatch-binding.json binding for ${seat} has the wrong agent_type`,
      );
    }
    if (
      binding.model !== MODEL_POLICY ||
      binding.reasoning_effort !== EFFORT_POLICY ||
      binding.context_tier !== "default" ||
      binding.communication !== "caveman-full-optional"
    ) {
      throw new Error(
        `dispatch-binding.json binding for ${seat} disagrees with the current ` +
        "panel dispatch policy",
      );
    }
  }
  return value.bindings;
}

const roundDir = resolve(dir);
let boundArtifacts;
try {
  boundArtifacts = readCompletionBoundArtifacts(roundDir);
} catch (cause) {
  fail(`invalid completion-bound round packet: ${cause.message}`);
}
let selectionCanonicalPath;
let verificationResultsCanonicalPath;
let approvalCanonicalPath;
try {
  selectionCanonicalPath = canonicalRoundPath(roundDir, selectionPath, "selection.json");
  canonicalRoundPath(roundDir, ledgerPath, "discovery-ledger.json");
  canonicalRoundPath(roundDir, responsesPath, "responses.json");
  verificationResultsCanonicalPath = canonicalRoundPath(
    roundDir,
    verificationResultsPath,
    "verification-results.json",
  );
  approvalCanonicalPath = canonicalRoundPath(roundDir, approvalPath, "approval.json");
} catch (cause) {
  fail(`non-canonical round input: ${cause.message}`);
}
if (errors.length) {
  for (const message of errors) console.error(`error: ${message}`);
  process.exit(1);
}
const boundJson = (relativePath, label) => {
  const bytes = boundArtifacts.artifacts.get(relativePath);
  if (!bytes) {
    fail(`completion marker does not bind ${relativePath}`);
    return { bytes: null, text: "", value: null };
  }
  try {
    return parseJsonBytes(
      bytes,
      join(roundDir, relativePath),
      label,
    );
  } catch (cause) {
    fail(cause.message);
    return { bytes, text: bytes.toString("utf8"), value: null };
  }
};
const addressArtifact = boundJson("address.json", "round address");
const candidateArtifact = boundJson("current-candidate.json", "current candidate address");
const selectionArtifact = boundJson("selection.json", "lifecycle selection");
const dispatchBindingArtifact = boundJson(
  "dispatch-binding.json",
  "roster-projected dispatch binding",
);
const discoveryLedgerArtifact = boundJson(
  "discovery-ledger.json",
  "immutable discovery ledger",
);
const responsesArtifact = boundJson("responses.json", "implementation responses");
const observedArtifact = readPostStageJson(
  join(roundDir, "observed.json"),
  "observed binding table",
);
const approvalArtifact = readPostStageJson(approvalCanonicalPath, "approval artifact");
const verificationResultsArtifact = readPostStageJson(
  verificationResultsCanonicalPath,
  "adapted verification results",
);
const address = addressArtifact.value;
const candidate = candidateArtifact.value;
const observed = observedArtifact.value;
const approval = approvalArtifact.value;
const approvalBytes = approvalArtifact.text;
const discoveryLedger = discoveryLedgerArtifact.value;
const discoveryLedgerBytes = discoveryLedgerArtifact.text;
const responses = responsesArtifact.value;
const responsesBytes = responsesArtifact.text;
const verificationResults = verificationResultsArtifact.value;
const verificationResultsBytes = verificationResultsArtifact.text;
let selection = null;
let dispatchBindings = null;
if (selectionArtifact.value !== null) {
  try {
    selection = validateSelection(selectionArtifact.value);
  } catch (cause) {
    fail(`invalid lifecycle selection at ${selectionCanonicalPath}: ${cause.message}`);
  }
}
if (
  selection &&
  (
    boundArtifacts.marker.lifecycle_id !== selection.lifecycle_id ||
    boundArtifacts.marker.selection_sha256 !== sha256(selectionArtifact.text)
  )
) {
  fail(
    "immutable panel completion marker does not bind the current verification " +
    "lifecycle and selection",
  );
}
if (selection && dispatchBindingArtifact.value !== null) {
  try {
    dispatchBindings = validateDispatchBinding(
      dispatchBindingArtifact.value,
      selection,
    );
  } catch (cause) {
    fail(`invalid roster-projected dispatch binding: ${cause.message}`);
  }
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
try {
  validateResponses(discoveryLedger, responses);
} catch (cause) {
  fail(`invalid implementation responses: ${cause.message}`);
}

let selectionBytes;
selectionBytes = selectionArtifact.text;
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
    selectionBytes,
    responseBytes: responsesBytes,
    verificationResultsBytes,
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
try {
  validateVerificationResultArtifact(verificationResults, {
    selection,
    ledger: discoveryLedger,
    ledger_bytes: discoveryLedgerBytes,
    selection_bytes: selectionBytes,
  });
} catch (cause) {
  fail(`invalid adapted verification results: ${cause.message}`);
}
if (approval && approval.discovery_ledger_sha256 !== sha256(discoveryLedgerBytes)) {
  fail("approval artifact is not bound to the immutable discovery ledger bytes");
}
if (approval && approval.response_sha256 !== sha256(responsesBytes)) {
  fail("approval artifact is not bound to the exact implementation response bytes");
}
if (approval && approval.verification_results_sha256 !== sha256(verificationResultsBytes)) {
  fail("approval artifact is not bound to the exact adapted verification-result bytes");
}
try {
  const expectedApproval = createApprovalArtifact({
    current_selection: selection,
    selection_bytes: selectionBytes,
    discovery_ledger: discoveryLedger,
    discovery_ledger_bytes: discoveryLedgerBytes,
    current_candidate: candidate,
    responses,
    responses_bytes: responsesBytes,
    verification_results: verificationResults,
    verification_results_bytes: verificationResultsBytes,
  });
  if (approvalBytes !== stableStringify(expectedApproval)) {
    fail(
      "approval artifact bytes do not match the exact selection, ledger, response, " +
      "and adapted verification-result inputs",
    );
  }
} catch (cause) {
  fail(`approval artifact does not recompute from canonical inputs: ${cause.message}`);
}

if (!candidate || typeof candidate !== "object" || Array.isArray(candidate)) {
  fail("current-candidate.json must be an object");
} else {
  for (const key of ["candidate_id", "content_id", "snapshot_sha256"]) {
    if (typeof candidate[key] !== "string" || !candidate[key]) {
      fail(`current-candidate.json is missing ${key}`);
    }
  }
}
const recordedSelectionPath =
  typeof address?.selection_path === "string"
    ? resolve(address.selection_path)
    : undefined;
if (
  address?.lifecycle_id !== selection.lifecycle_id ||
  recordedSelectionPath !== selectionCanonicalPath
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
  fail("approval artifact current candidate disagrees with staged current-candidate.json");
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
const adaptedVerificationResults =
  verificationResults && Array.isArray(verificationResults.results)
    ? Object.fromEntries(verificationResults.results.map((result) => [result.seat, result]))
    : verificationResults?.results ?? {};
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
const stagedDefinitionDigests = {};
for (const seat of roster) {
  const relativePath = `agent-definitions/panel-${seat}.agent.md`;
  const bytes = boundArtifacts.artifacts.get(relativePath);
  const expectedDigest = boundArtifacts.marker.artifact_sha256?.[relativePath];
  const expectedBytes = boundArtifacts.marker.artifact_bytes?.[relativePath];
  if (!Buffer.isBuffer(bytes)) {
    fail(
      `immutable panel completion marker has no staged agent definition for ${seat}`,
    );
    continue;
  }
  if (
    typeof expectedDigest !== "string" ||
    !/^[0-9a-f]{64}$/u.test(expectedDigest) ||
    expectedBytes !== bytes.length
  ) {
    fail(
      `immutable panel completion marker has an invalid binding for ${relativePath}`,
    );
    continue;
  }
  const actualDigest = createHash("sha256").update(bytes).digest("hex");
  if (actualDigest !== expectedDigest) {
    fail(
      `staged agent definition digest for ${seat} does not match .complete`,
    );
    continue;
  }
  stagedDefinitionDigests[seat] = actualDigest;
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
  const verdictArtifact = readPostStageJson(
    join(verdictDir, `${role}.json`),
    `verdict for ${role}`,
    MAX_VERDICT_BYTES,
  );
  const verdict = verdictArtifact.value;
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
  try {
    const adapted = adaptVerificationVerdict(verdict, {
      seat: role,
      issue_ids: discoveryLedger.issues.map((issue) => issue.id),
    });
    const recorded = adaptedVerificationResults[role];
    if (!recorded || stableStringify(adapted) !== stableStringify(recorded)) {
      fail(
        `verdict ${role} does not match the exact adapted verification-result bytes; ` +
        "adapt verification again before generating records",
      );
    }
  } catch (cause) {
    fail(`verdict ${role} cannot be adapted to the exact verification result: ${cause.message}`);
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
  const observedKeysForSeat = Object.keys(observedBinding).sort();
  const expectedObservedKeys = OBSERVED_BINDING_KEYS.slice().sort();
  if (
    observedKeysForSeat.length !== expectedObservedKeys.length ||
    observedKeysForSeat.some((key, index) => key !== expectedObservedKeys[index])
  ) {
    const unknown = observedKeysForSeat.filter(
      (key) => !expectedObservedKeys.includes(key),
    );
    const missing = expectedObservedKeys.filter(
      (key) => !observedKeysForSeat.includes(key),
    );
    fail(
      `observed.json ${role} must contain exactly the nine documented fields; ` +
      `unknown [${unknown.join(", ")}], missing [${missing.join(", ")}]`,
    );
  }
  for (const key of OBSERVED_BINDING_KEYS) {
    if (
      typeof observedBinding[key] !== "string" ||
      observedBinding[key].length === 0
    ) {
      fail(`observed.json ${role}: ${key} is required`);
    }
  }
  const expectedBinding = dispatchBindings[role];
  for (const key of DISPATCH_BINDING_KEYS) {
    if (observedBinding[key] !== expectedBinding[key]) {
      fail(
        `observed.json ${role}: ${key} "${observedBinding[key]}" does not ` +
        `match the selected binding from the completion-bound dispatch policy; ` +
        `policy pins "${expectedBinding[key]}"`,
      );
    }
  }
  if (
    typeof observedBinding.agent_definition_sha256 === "string" &&
    !/^[0-9a-f]{64}$/u.test(observedBinding.agent_definition_sha256)
  ) {
    fail(
      `observed.json ${role}: agent_definition_sha256 must be a 64-character hexadecimal SHA-256`,
    );
  }
  if (
    stagedDefinitionDigests[role] === undefined ||
    observedBinding.agent_definition_sha256 !== stagedDefinitionDigests[role]
  ) {
    fail(
      `observed.json ${role}: agent definition digest does not match the immutable ` +
      `staged agent-definitions/panel-${role}.agent.md digest`,
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

function writeRecordFamilyCreateOrCompare(directory, entries) {
  const expected = new Map(
    entries
      .map((entry) => [entry.name, Buffer.from(entry.bytes)])
      .sort(([left], [right]) => left.localeCompare(right)),
  );
  const compare = () => {
    if (!statSync(directory).isDirectory()) {
      throw new Error(`record family at ${directory} is not a directory`);
    }
    const actualNames = readdirSync(directory).sort();
    const expectedNames = [...expected.keys()];
    if (
      actualNames.length !== expectedNames.length ||
      actualNames.some((name, index) => name !== expectedNames[index])
    ) {
      throw new Error(`record family at ${directory} is incomplete or has extra entries`);
    }
    for (const name of expectedNames) {
      if (
        !readLimitedBytes(
          join(directory, name),
          `existing record ${name}`,
          MAX_RECORD_BYTES,
        ).equals(expected.get(name))
      ) {
        throw new Error(`conflicting generated record bytes at ${join(directory, name)}`);
      }
    }
  };
  mkdirSync(dirname(directory), { recursive: true });
  if (existsSync(directory)) {
    compare();
    return;
  }
  const temporary = `${directory}.stage-${process.pid}-${Date.now()}`;
  mkdirSync(temporary);
  try {
    for (const [name, bytes] of expected) {
      writeFileSync(join(temporary, name), bytes, { flag: "wx" });
    }
    try {
      renameSync(temporary, directory);
    } catch (cause) {
      if (!existsSync(directory)) throw cause;
      compare();
    }
  } finally {
    if (existsSync(temporary)) {
      rmSync(temporary, { recursive: true, force: true });
    }
  }
}

const outDir = join(dir, "records");
const pendingWrites = records.map((record) => ({
  name: `${record.role}.json`,
  bytes: `${JSON.stringify(record, null, 2)}\n`,
}));
try {
  writeRecordFamilyCreateOrCompare(outDir, pendingWrites);
} catch (cause) {
  fail(
    `record set publication stopped before replacement: ` +
    `${cause.message}. Retry only after restoring the exact byte-identical ` +
    `record family for the same inputs, or use a new qualified round.`,
  );
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
