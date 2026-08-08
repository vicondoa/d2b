#!/usr/bin/env node
/*
 * The standard Copilot panel lifecycle.
 *
 * This is deliberately a small data helper rather than a runner. Copilot
 * dispatch, judgement about duplicate findings, and ordinary repository
 * controls remain outside this file. The helper validates the boundaries
 * between those steps and renders their JSON inputs deterministically.
 */

import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { TextDecoder } from "node:util";
import {
  existsSync,
  linkSync,
  lstatSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { basename, dirname, join, posix, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const DEFAULT_SELECTION_TABLE = join(HERE, "..", "selection-table.json");

export const SELECTION_SCHEMA_VERSION = 1;
export const SELECTION_TABLE_VERSION = 2;
export const LIFECYCLE_SELECTION_ARTIFACT =
  "d2b-panel/lifecycle-selection";
export const DISCOVERY_REQUEST_ARTIFACT = "d2b-panel/discovery-request";
export const DISCOVERY_RESULT_ARTIFACT = "d2b-panel/discovery-result";
export const LEDGER_ARTIFACT = "d2b-panel/issue-ledger";
export const RESPONSE_ARTIFACT = "d2b-panel/implementation-responses";
export const VERIFICATION_ARTIFACT = "d2b-panel/verification";
export const LEGACY_IMPORT_ARTIFACT = "d2b-panel/legacy-import";
export const APPROVAL_ARTIFACT = "d2b-panel/approval";
export const METRICS_ARTIFACT = "d2b-panel/metrics";

export const SEVERITIES = Object.freeze([
  "BLOCKER",
  "MAJOR",
  "MINOR",
  "NIT",
]);
export const DISPOSITIONS = Object.freeze([
  "Fixed",
  "Intentionally rejected",
  "Deferred",
  "Withdrawn",
  "Invalid",
]);
export const VERIFICATION_STATUSES = Object.freeze([
  "resolved",
  "addressed",
  "fixed",
  "verified",
  "accepted",
  "invalid",
  "withdrawn",
  "deferred",
  "not_applicable",
  "open",
  "blocked",
  "unresolved",
  "regression",
]);

export const LEGACY_ROSTER = Object.freeze([
  "software",
  "test",
  "nixos",
  "networking",
  "security",
  "rust",
  "product",
  "docs",
  "observability",
  "kernel",
]);
const LEGACY_MODEL_POLICY = "gemini-3.1-pro-preview";
const LEGACY_EFFORT_POLICY = "high";

const SEVERITY_RANK = Object.freeze({
  BLOCKER: 4,
  MAJOR: 3,
  MINOR: 2,
  NIT: 1,
});

const error = (message) => {
  throw new Error(message);
};

let temporaryCounter = 0;

/*
 * Rust's ordered sets compare the UTF-8 representation of valid strings.
 * JavaScript's default Array#sort compares UTF-16 code units, which puts a
 * non-BMP string in a different position from the equivalent Rust ordering.
 * Keep one comparator for every path or signal array that crosses the
 * lifecycle artifact boundary.
 */
function utf8Bytes(value, label) {
  if (typeof value !== "string") {
    error(`${label} must be a string`);
  }
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    if (code >= 0xd800 && code <= 0xdbff) {
      const next = value.charCodeAt(index + 1);
      if (next < 0xdc00 || next > 0xdfff) {
        error(`${label} is not representable as UTF-8`);
      }
      index += 1;
    } else if (code >= 0xdc00 && code <= 0xdfff) {
      error(`${label} is not representable as UTF-8`);
    }
  }
  return Buffer.from(value, "utf8");
}

const compareUtf8 = (left, right) =>
  utf8Bytes(left, "ordered string").compare(utf8Bytes(right, "ordered string"));

const sortUtf8 = (values) => [...values].sort(compareUtf8);
const CONTROL_CHARACTER_PATTERN = /[\u0000-\u001f\u007f-\u009f]/u;

const isPlainObject = (value) =>
  value !== null &&
  typeof value === "object" &&
  !Array.isArray(value);

const nonBlank = (value, label) => {
  if (typeof value !== "string" || value.trim() === "") {
    error(`${label} must be a non-blank string`);
  }
  return value;
};

const optionalString = (value, label) => {
  if (value === undefined) return undefined;
  return nonBlank(value, label);
};

const sortedObject = (value) => {
  if (Array.isArray(value)) return value.map(sortedObject);
  if (!isPlainObject(value)) return value;
  return Object.fromEntries(
    Object.keys(value)
      .sort()
      .map((key) => [key, sortedObject(value[key])]),
  );
};

export const stableStringify = (value) =>
  `${JSON.stringify(sortedObject(value), null, 2)}\n`;

export const sha256 = (value) =>
  createHash("sha256")
    .update(typeof value === "string" ? value : stableStringify(value))
    .digest("hex");

const readJson = (path, label = path) => {
  if (!existsSync(path)) error(`missing ${label} at ${path}`);
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch (cause) {
    error(`invalid ${label} at ${path}: ${cause.message}`);
  }
};

export function changedPathsFromGitRange(range, cwd = process.cwd()) {
  nonBlank(range, "git range");
  if (CONTROL_CHARACTER_PATTERN.test(range)) {
    error("git range contains a control character");
  }
  let output;
  try {
    output = execFileSync(
      "git",
      ["diff", "--name-only", "-z", "--diff-filter=ACDMRTUXB", range],
      { cwd },
    );
  } catch (cause) {
    error(`cannot derive changed paths from git range ${range}: ${cause.message}`);
  }
  if (!Buffer.isBuffer(output)) {
    error("git changed-path output was not returned as bytes");
  }
  if (output.length === 0) return [];
  if (output[output.length - 1] !== 0) {
    error("git changed-path output is not NUL-terminated");
  }
  const decoder = new TextDecoder("utf-8", { fatal: true });
  const paths = [];
  let start = 0;
  for (let index = 0; index < output.length; index += 1) {
    if (output[index] !== 0) continue;
    if (index === start) {
      error("git changed-path output contains an unrepresentable NUL path");
    }
    let path;
    try {
      path = decoder.decode(output.subarray(start, index));
    } catch (cause) {
      error(`git changed-path output contains invalid UTF-8: ${cause.message}`);
    }
    utf8Bytes(path, "git changed path");
    if (CONTROL_CHARACTER_PATTERN.test(path)) {
      error("git changed path contains an unrepresentable control character");
    }
    paths.push(path);
    start = index + 1;
  }
  return sortUtf8([...new Set(paths)]);
}

/*
 * Every generated file is create-or-compare. A caller can safely retry a
 * command, but cannot silently replace an artifact that another step already
 * consumed.
 */
export function writeCreateOrCompare(path, value) {
  const expected = stableStringify(value);
  mkdirSync(dirname(path), { recursive: true });
  if (existsSync(path)) {
    const actual = readFileSync(path, "utf8");
    if (actual !== expected) {
      error(
        `conflicting generated bytes at ${path}; refusing to overwrite the existing artifact`,
      );
    }

    return { path, created: false, bytes: expected };
  }
  const temporary = `${path}.${process.pid}.${temporaryCounter += 1}.tmp`;
  try {
    writeFileSync(temporary, expected, { encoding: "utf8", flag: "wx" });
    /*
     * A hard link is the no-replace commit primitive. Unlike rename, it
     * cannot replace a file created by a concurrent retry. The temporary name
     * is never the published artifact, so a failed write cannot look complete
     * to a reader.
     */
    linkSync(temporary, path);
    unlinkSync(temporary);
    return { path, created: true, bytes: expected };
  } catch (cause) {
    try {
      unlinkSync(temporary);
    } catch {
      // The temporary file may not have been created.
    }
    if (cause.code === "EEXIST" && existsSync(path)) {
      const actual = readFileSync(path, "utf8");
      if (actual !== expected) {
        error(
          `conflicting generated bytes at ${path}; refusing to overwrite the existing artifact`,
        );
      }
      return { path, created: false, bytes: expected };
    }
    throw cause;
  }
}

let atomicDirectoryMoveAvailable;

function requireAtomicDirectoryMove() {
  if (atomicDirectoryMoveAvailable !== undefined) {
    if (!atomicDirectoryMoveAvailable) {
      error(
        "directory publication requires GNU mv with --no-clobber and " +
        "--no-target-directory; the atomic no-clobber primitive is unavailable",
      );
    }
    return;
  }
  try {
    const help = execFileSync("mv", ["--help"], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });
    atomicDirectoryMoveAvailable =
      help.includes("--no-clobber") && help.includes("--no-target-directory");
  } catch {
    atomicDirectoryMoveAvailable = false;
  }
  if (!atomicDirectoryMoveAvailable) {
    error(
      "directory publication requires GNU mv with --no-clobber and " +
      "--no-target-directory; the atomic no-clobber primitive is unavailable",
    );
  }
}

function atomicNoClobberDirectoryMove(source, destination) {
  requireAtomicDirectoryMove();
  try {
    execFileSync(
      "mv",
      ["--no-clobber", "--no-target-directory", "--", source, destination],
      { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] },
    );
  } catch (cause) {
    error(
      `atomic no-clobber directory publication failed for ${destination}: ` +
      `${cause.message}`,
    );
  }
}

/*
 * A directory is the publication unit for an artifact family. Build every
 * member in a complete sibling temporary directory, claim a separate sibling
 * claim directory, and publish with Linux's atomic no-clobber move. The
 * destination therefore changes from absent to complete, never from absent to
 * partially populated. A claim left by a crashed publisher is not reclaimed
 * implicitly: its error names the cleanup needed before retrying.
 */
export function writeDirectoryCreateOrCompare(directory, entries) {
  if (!Array.isArray(entries) || entries.length === 0) {
    error("directory publication requires at least one artifact");
  }
  const expected = new Map();
  for (const entry of entries) {
    if (!isPlainObject(entry)) error("directory publication entries must be objects");
    const name = safePathPart(entry.name, "directory publication filename");
    if (expected.has(name)) error(`directory publication repeats ${name}`);
    if (typeof entry.bytes !== "string" && !Buffer.isBuffer(entry.bytes)) {
      error(`directory publication ${name} bytes must be a string or Buffer`);
    }
    expected.set(name, entry.bytes);
  }
  const expectedNames = [...expected.keys()].sort();

  const pathExists = (path) => {
    try {
      lstatSync(path);
      return true;
    } catch (cause) {
      if (cause.code === "ENOENT") return false;
      throw cause;
    }
  };

  const compareExisting = (path) => {
    if (!pathExists(path) || !lstatSync(path).isDirectory()) {
      error(`existing artifact family at ${path} is not a directory`);
    }
    const actualEntries = readdirSync(path, { withFileTypes: true })
      .sort((left, right) => left.name.localeCompare(right.name));
    const actualNames = actualEntries.map((entry) => entry.name);
    if (
      actualNames.length !== expectedNames.length ||
      actualNames.some((name, index) => name !== expectedNames[index])
    ) {
      error(
        `existing artifact family at ${path} is incomplete or has extra entries; ` +
        `expected [${expectedNames.join(", ")}], found [${actualNames.join(", ")}]`,
      );
    }
    for (const entry of actualEntries) {
      if (!entry.isFile()) {
        error(`existing artifact family entry ${join(path, entry.name)} is not a regular file`);
      }
      const actual = readFileSync(join(path, entry.name));
      const expectedBytes = expected.get(entry.name);
      const expectedBuffer = Buffer.isBuffer(expectedBytes)
        ? expectedBytes
        : Buffer.from(expectedBytes);
      if (!actual.equals(expectedBuffer)) {
        error(`conflicting generated bytes at ${join(path, entry.name)}; refusing to overwrite`);
      }
    }
    return { path, created: false };
  };

  mkdirSync(dirname(directory), { recursive: true });
  const temporary = mkdtempSync(
    join(dirname(directory), `.${basename(directory)}.stage-${process.pid}-`),
  );
  const claim = `${directory}.claim`;
  let claimOwned = false;
  try {
    for (const name of expectedNames) {
      writeFileSync(join(temporary, name), expected.get(name), { flag: "wx" });
    }
    const stagedNames = readdirSync(temporary).sort();
    if (
      stagedNames.length !== expectedNames.length ||
      stagedNames.some((name, index) => name !== expectedNames[index])
    ) {
      error(`staged artifact family at ${temporary} is incomplete before publication`);
    }

    try {
      mkdirSync(claim);
      claimOwned = true;
    } catch (cause) {
      if (cause.code === "EEXIST") {
        if (pathExists(directory)) return compareExisting(directory);
        error(
          `sibling publication claim ${claim} already exists while ` +
          `destination ${directory} is absent; it may be stale. ` +
          `Clean up the stale claim before retrying: rm -rf -- '${claim}'`,
        );
      }
      throw cause;
    }

    if (pathExists(directory)) {
      return compareExisting(directory);
    }

    /*
     * The staged directory is complete and the claim serializes compliant
     * publishers. The no-clobber move is still required: a publisher that
     * does not take the claim must not win a check-then-rename race.
     */
    atomicNoClobberDirectoryMove(temporary, directory);
    if (pathExists(temporary)) {
      /*
       * GNU mv leaves the source in place when --no-clobber declines an
       * existing destination. Compare only after that atomic decision.
       */
      return compareExisting(directory);
    }
    if (!pathExists(directory)) {
      error(
        `atomic directory publication moved ${temporary} but destination ` +
        `${directory} is absent`,
      );
    }
    return { path: directory, created: true };
  } finally {
    rmSync(temporary, { recursive: true, force: true });
    if (claimOwned) {
      rmSync(claim, { recursive: true, force: true });
    }
  }
}

function assertExactKeys(value, expected, label) {
  if (!isPlainObject(value)) error(`${label} must be a JSON object`);
  const actual = Object.keys(value).sort();
  const wanted = [...expected].sort();
  if (actual.length !== wanted.length || actual.some((key, i) => key !== wanted[i])) {
    error(
      `${label} has fields [${actual.join(", ")}]; expected exactly [${wanted.join(", ")}]`,
    );
  }
}

function assertDigest(value, label) {
  if (typeof value !== "string" || !/^[0-9a-f]{64}$/.test(value)) {
    error(`${label} must be a 64-character hexadecimal SHA-256`);
  }
  return value;
}

function safePathPart(value, label) {
  nonBlank(value, label);
  if (value === "." || value === ".." || /[\\/]/.test(value)) {
    error(`${label} must be a single path component`);
  }
  return value;
}

function candidateAddress(input) {
  const candidate =
    input?.current_candidate ??
    input?.currentCandidate ??
    input?.candidate ??
    input?.full_candidate ??
    input?.fullCandidate ??
    input;
  if (!isPlainObject(candidate)) error("candidate address must be an object");
  const address = {
    program: nonBlank(candidate.program, "candidate.program"),
    wave: nonBlank(candidate.wave, "candidate.wave"),
    candidate_id: safePathPart(candidate.candidate_id, "candidate.candidate_id"),
    content_id: nonBlank(candidate.content_id, "candidate.content_id"),
    snapshot_sha256: assertDigest(
      candidate.snapshot_sha256,
      "candidate.snapshot_sha256",
    ),
  };
  return address;
}

function candidateInputs(input) {
  const candidate =
    input?.current_candidate ??
    input?.currentCandidate ??
    input?.candidate ??
    input;
  const classification = input?.classification_inputs ??
    candidate?.classification_inputs ??
    {};
  const paths = input?.changed_paths ??
    input?.changedPaths ??
    input?.paths ??
    candidate?.changed_paths ??
    candidate?.changedPaths ??
    candidate?.paths ??
    classification.changed_paths ??
    [];
  if (!Array.isArray(paths) || paths.some((path) => typeof path !== "string")) {
    error("changed_paths must be an array of strings");
  }
  for (const path of paths) {
    utf8Bytes(path, "changed path");
    if (CONTROL_CHARACTER_PATTERN.test(path)) {
      error("changed paths must not contain control characters");
    }
  }
  const signals = input?.signals ??
    input?.content_signals ??
    input?.contentSignals ??
    candidate?.signals ??
    candidate?.content_signals ??
    candidate?.contentSignals ??
  classification.signals ??
  [];
  if (!Array.isArray(signals) || signals.some((signal) => typeof signal !== "string")) {
    error("signals must be an array of strings");
  }
  for (const signal of signals) {
    utf8Bytes(signal, "signal");
    if (CONTROL_CHARACTER_PATTERN.test(signal)) {
      error("signals must not contain control characters");
    }
  }
  const candidateClass = input?.candidate_class ??
    input?.candidateClass ??
    input?.classification ??
    candidate?.candidate_class ??
    candidate?.candidateClass ??
    candidate?.classification ??
    classification.candidate_class ??
    undefined;
  if (candidateClass !== undefined) nonBlank(candidateClass, "candidate_class");
  const ambiguous =
    input?.ambiguous === true ||
    input?.ambiguity_widened === true ||
    candidate?.ambiguous === true ||
    candidate?.ambiguity === true ||
    candidateClass === "ambiguous";
  return {
    changed_paths: sortUtf8([...new Set(paths)]),
    signals: sortUtf8([
      ...new Set(signals.map((signal) => signal.trim().toLowerCase())),
    ]),
    candidate_class: candidateClass ?? undefined,
    ambiguous,
  };
}

function globRegex(pattern) {
  let source = "^";
  for (let i = 0; i < pattern.length; i += 1) {
    const char = pattern[i];
    if (char === "*" && pattern[i + 1] === "*") {
      i += 1;
      if (pattern[i + 1] === "/") {
        i += 1;
        source += "(?:.*/)?";
      } else {
        source += ".*";
      }
    } else if (char === "*") {
      source += "[^/]*";
    } else if (char === "?") {
      source += "[^/]";
    } else {
      source += char.replace(/[.+^${}()|[\]\\]/g, "\\$&");
    }
  }
  return new RegExp(`${source}$`, "i");
}

function pathPatternMatches(path, pattern) {
  return globRegex(pattern).test(path);
}

function triggerMatches(trigger, inputs) {
  if (!isPlainObject(trigger)) error("selection-table trigger must be an object");
  if (trigger.kind === "always") return true;
  if (trigger.kind === "path") {
    if (!Array.isArray(trigger.patterns)) {
      error("path trigger must contain patterns");
    }
    return inputs.changed_paths.some((path) =>
      trigger.patterns.some((pattern) => pathPatternMatches(path, pattern)),
    );
  }
  if (trigger.kind === "signal") {
    if (!Array.isArray(trigger.values)) {
      error("signal trigger must contain values");
    }
    return trigger.values.some((value) =>
      inputs.signals.includes(String(value).toLowerCase()),
    );
  }
  error(`unknown selection-table trigger kind "${trigger.kind}"`);
}

function validateTable(table) {
  if (!isPlainObject(table)) error("selection table must be an object");
  if (table.artifact_kind !== "d2b-panel/selection-table") {
    error("selection table has an unexpected artifact_kind");
  }
  if (table.selection_table_version !== SELECTION_TABLE_VERSION) {
    error(
      `selection table version ${table.selection_table_version} is not supported; expected ${SELECTION_TABLE_VERSION}`,
    );
  }
  for (const key of ["mandatory_seats", "optional_seats", "fill_order"]) {
    if (!Array.isArray(table[key]) || table[key].some((seat) => typeof seat !== "string")) {
      error(`selection table ${key} must be an array of seat names`);
    }
  }
  const all = [...table.mandatory_seats, ...table.optional_seats];
  if (new Set(all).size !== all.length) error("selection table repeats a seat");
  if (
    table.fill_order.length !== table.optional_seats.length ||
    table.fill_order.some((seat, index) => seat !== table.optional_seats[index])
  ) {
    error("selection table fill_order must contain every optional seat in order");
  }
  if (!isPlainObject(table.floors)) error("selection table floors must be an object");
  for (const candidateClass of ["code", "configuration", "documentation", "ambiguous"]) {
    if (!Number.isInteger(table.floors[candidateClass]) || table.floors[candidateClass] < table.mandatory_seats.length) {
      error(`selection table floor for ${candidateClass} is invalid`);
    }
  }
  if (!isPlainObject(table.seats)) error("selection table seats must be an object");
  const seatKeys = Object.keys(table.seats).sort();
  const expectedSeatKeys = [...all].sort();
  if (
    seatKeys.length !== expectedSeatKeys.length ||
    seatKeys.some((seat, index) => seat !== expectedSeatKeys[index])
  ) {
    error("selection table seats must contain exactly the mandatory and optional seats");
  }
  for (const seat of all) {
    const definition = table.seats[seat];
    if (!isPlainObject(definition)) error(`selection table has no definition for ${seat}`);
    if (definition.class !== (table.mandatory_seats.includes(seat) ? "mandatory" : "optional")) {
      error(`selection table class for ${seat} disagrees with its seat class`);
    }
    nonBlank(definition.focus, `selection table focus for ${seat}`);
    if (!Array.isArray(definition.triggers)) error(`selection table triggers for ${seat} must be an array`);
    if (!isPlainObject(definition.profiles)) error(`selection table profiles for ${seat} must be an object`);
    for (const trigger of definition.triggers) {
      if (!isPlainObject(trigger) || typeof trigger.kind !== "string") {
        error(`selection table trigger for ${seat} is malformed`);
      }
      if (!["always", "path", "signal"].includes(trigger.kind)) {
        error(`selection table trigger for ${seat} has unknown kind ${trigger.kind}`);
      }
      for (const key of ["patterns", "values"]) {
        if (trigger[key] !== undefined &&
            (!Array.isArray(trigger[key]) ||
             trigger[key].some((value) => typeof value !== "string" || value.trim() === ""))) {
          error(`selection table trigger ${seat}/${trigger.kind} ${key} is malformed`);
        }
      }
    }
    for (const [profile, profileDefinition] of Object.entries(definition.profiles)) {
      if (!isPlainObject(profileDefinition)) {
        error(`selection table profile ${seat}/${profile} is malformed`);
      }
      for (const key of ["paths", "signals"]) {
        if (profileDefinition[key] !== undefined &&
            (!Array.isArray(profileDefinition[key]) ||
             profileDefinition[key].some((value) => typeof value !== "string" || value.trim() === ""))) {
          error(`selection table profile ${seat}/${profile} ${key} is malformed`);
        }
      }
    }
  }
  return table;
}

export function readSelectionTable(path = DEFAULT_SELECTION_TABLE) {
  return validateTable(readJson(path, "selection table"));
}

function inferCandidateClass(inputs) {
  if (inputs.ambiguous) return "ambiguous";
  if (inputs.candidate_class === "documentation" || inputs.candidate_class === "docs") {
    if (inputs.changed_paths.some((path) => !isDocumentationPath(path))) {
      error(
        "candidate_class documentation cannot narrow actual non-documentation paths",
      );
    }
    return "documentation";
  }
  if (
    inputs.candidate_class === "configuration" ||
    inputs.candidate_class === "config"
  ) {
    return "configuration";
  }
  if (inputs.candidate_class === "code") return "code";
  const paths = inputs.changed_paths;
  if (
    paths.length > 0 &&
    paths.every((path) => isDocumentationPath(path))
  ) {
    return "documentation";
  }
  return "code";
}

function isDocumentationPath(path) {
  const asciiLower = path.replace(/[A-Z]/g, (character) =>
    character.toLowerCase(),
  );
  if (asciiLower.startsWith("docs/") || asciiLower.startsWith("changelog.d/")) {
    return true;
  }
  if (asciiLower.includes("/")) return false;
  if (
    asciiLower === "readme" ||
    asciiLower.startsWith("readme.") ||
    asciiLower === "changelog" ||
    asciiLower.startsWith("changelog.")
  ) {
    return true;
  }
  return [".md", ".mdx", ".rst", ".txt"].some((suffix) =>
    asciiLower.endsWith(suffix) && asciiLower.length > suffix.length,
  );
}

function candidateClassPrecedence(classes) {
  for (const candidateClass of ["ambiguous", "code", "configuration", "documentation"]) {
    if (classes.includes(candidateClass)) return candidateClass;
  }
  error("candidate classification requires at least one nested classification");
}

function seatHasTrigger(table, seat, inputs) {
  return table.seats[seat].triggers.some((trigger) => triggerMatches(trigger, inputs));
}

function profilesForSeat(table, seat, inputs) {
  const profiles = [];
  for (const [profile, definition] of Object.entries(table.seats[seat].profiles)) {
    if (!isPlainObject(definition)) error(`profile ${seat}/${profile} must be an object`);
    const pathMatch =
      Array.isArray(definition.paths) &&
      inputs.changed_paths.some((path) =>
        definition.paths.some((pattern) => pathPatternMatches(path, pattern)),
      );
    const signalMatch =
      Array.isArray(definition.signals) &&
      definition.signals.some((signal) =>
        inputs.signals.includes(String(signal).toLowerCase()),
      );
    if (pathMatch || signalMatch) profiles.push(profile);
  }
  return profiles.sort();
}

function seatOrder(table) {
  return [...table.mandatory_seats, ...table.fill_order];
}

export function selectRoster(input, options = {}) {
  const table = options.table ?? readSelectionTable(options.table_path);
  validateTable(table);
  const inputs = candidateInputs(input);
  const candidateClass = inferCandidateClass(inputs);
  const triggeredOptional = table.optional_seats.filter((seat) =>
    seatHasTrigger(table, seat, inputs),
  );
  const targetFloor = table.floors[candidateClass];
  const selected = [...table.mandatory_seats];
  const floorFilled = [];
  for (const seat of table.fill_order) {
    if (triggeredOptional.includes(seat) || selected.length < targetFloor) {
      if (!selected.includes(seat)) {
        selected.push(seat);
        if (!triggeredOptional.includes(seat)) floorFilled.push(seat);
      }
    }
  }
  if (selected.length < targetFloor) {
    error(
      `selection table cannot fill ${candidateClass} floor ${targetFloor}; selected ${selected.length} seats`,
    );
  }
  const profiles = Object.fromEntries(
    selected.map((seat) => [seat, profilesForSeat(table, seat, inputs)]),
  );
  return {
    table_version: table.selection_table_version,
    candidate_class: candidateClass,
    classification_inputs: {
      changed_paths: inputs.changed_paths,
      signals: inputs.signals,
      candidate_class: candidateClass,
      ambiguous: inputs.ambiguous,
    },
    ambiguity_widened: inputs.ambiguous,
    mandatory_seats: [...table.mandatory_seats],
    triggered_optional: triggeredOptional,
    floor_filled: floorFilled,
    profiles,
    roster: selected,
  };
}

export function selectLifecycleRoster(input, options = {}) {
  const table = options.table ?? readSelectionTable(options.table_path);
  const fullCandidate = input.full_candidate ?? input.fullCandidate ?? input.candidate ?? input;
  const full = selectRoster(fullCandidate, { table });
  const deltaInput = input.fix_delta ?? input.fixDelta ?? input.delta;
  const delta = deltaInput
    ? selectRoster(deltaInput, { table })
    : {
        roster: [],
        profiles: {},
        triggered_optional: [],
        floor_filled: [],
        candidate_class: full.candidate_class,
        ambiguity_widened: false,
        classification_inputs: {
          changed_paths: [],
          signals: [],
          candidate_class: full.candidate_class,
          ambiguous: false,
        },
      };
  const prior = input.previous_roster ?? input.previousRoster;
  const roster = unionRosters(
    prior ? [prior, full.roster, ...(delta.roster.length ? [delta.roster] : [])] :
      [full.roster, ...(delta.roster.length ? [delta.roster] : [])],
    table,
  );
  const profiles = Object.fromEntries(
    roster.map((seat) => [
      seat,
      [...new Set([
        ...(full.profiles[seat] ?? []),
        ...(delta.profiles[seat] ?? []),
      ])].sort(),
    ]),
  );
  return {
    full,
    delta,
    roster,
    profiles,
    widened: delta.roster.some((seat) => !full.roster.includes(seat)),
  };
}

function validateRoster(roster, table, label = "roster") {
  if (!Array.isArray(roster) || roster.some((seat) => typeof seat !== "string")) {
    error(`${label} must be an array of seat names`);
  }
  const known = new Set([...table.mandatory_seats, ...table.optional_seats]);
  const seen = new Set();
  for (const seat of roster) {
    if (!known.has(seat)) error(`${label} contains unknown seat "${seat}"`);
    if (seen.has(seat)) error(`${label} contains duplicate seat "${seat}"`);
    seen.add(seat);
  }
  for (const seat of table.mandatory_seats) {
    if (!seen.has(seat)) error(`${label} omits mandatory seat "${seat}"`);
  }
  return [...roster];
}

/*
 * `validateSelection` checks the shape of an artifact. This second check is
 * deliberately derived from the same table as selection: a caller must not
 * be able to make a well-shaped artifact that omits an optional trigger or
 * invents a profile. Widened verification rosters are accepted, but the
 * mandatory and currently-triggered seats and profiles are never optional.
 */
export function validateSelectionAgainstTable(
  selection,
  table = readSelectionTable(),
) {
  validateTable(table);
  const inputs = selection.classification_inputs;
  const expected = selectRoster(
    {
      changed_paths: inputs.changed_paths,
      signals: inputs.signals,
      candidate_class: selection.candidate_class,
      ambiguous: inputs.ambiguous === true || selection.ambiguity_widened === true,
    },
    { table },
  );
  const selected = new Set(selection.roster);
  for (const seat of table.mandatory_seats) {
    if (!selected.has(seat)) {
      error(`selection roster omits mandatory seat "${seat}"`);
    }
  }
  for (const seat of expected.triggered_optional) {
    if (!selected.has(seat)) {
      error(`selection roster omits triggered seat "${seat}"`);
    }
  }
  if (selection.roster.length < table.floors[selection.candidate_class]) {
    error(
      `selection roster has ${selection.roster.length} seats but ` +
      `${selection.candidate_class} requires floor ${table.floors[selection.candidate_class]}`,
    );
  }
  for (const seat of expected.roster) {
    for (const profile of expected.profiles[seat] ?? []) {
      if (!selection.profiles[seat]?.includes(profile)) {
        error(`selection profile ${seat}/${profile} is missing for its classification inputs`);
      }
    }
  }
  return {
    expected_roster: expected.roster,
    triggered_optional: expected.triggered_optional,
    expected_profiles: expected.profiles,
  };
}

export function validateMonotonicRoster(
  previousRoster,
  nextRoster,
  table = readSelectionTable(),
) {
  const previous = validateRoster(previousRoster, table, "previous roster");
  const next = validateRoster(nextRoster, table, "next roster");
  const nextSet = new Set(next);
  for (const seat of previous) {
    if (!nextSet.has(seat)) {
      error(
        `roster narrowing removed "${seat}"; keep the seat in this lifecycle or start a new lifecycle`,
      );
    }
  }
  return true;
}

export function unionRosters(rosters, table = readSelectionTable()) {
  if (!Array.isArray(rosters) || rosters.length === 0) {
    error("at least one roster is required for a lifecycle union");
  }
  const seats = new Set();
  for (const roster of rosters) {
    validateRoster(roster, table);
    roster.forEach((seat) => seats.add(seat));
  }
  return seatOrder(table).filter((seat) => seats.has(seat));
}

function selectionPath(root, lifecycleId, candidateId, snapshotSha256, phase = "discovery") {
  safePathPart(lifecycleId, "lifecycle_id");
  safePathPart(candidateId, "candidate_id");
  assertDigest(snapshotSha256, "snapshot_sha256");
  const phasePart = phase === "verification" ? ["verification"] : [];
  return join(
    root,
    ".scratch",
    "panel",
    lifecycleId,
    "selections",
    ...phasePart,
    candidateId,
    `${snapshotSha256}.json`,
  );
}

export function selectionDigest(path) {
  if (!existsSync(path)) error(`missing lifecycle selection at ${path}`);
  return sha256(readFileSync(path, "utf8"));
}

export function candidateFromSelection(selection, options = {}) {
  const table = options.table ?? readSelectionTable(options.table_path);
  validateSelection(selection, table);
  return sortedObject({
    program: selection.program,
    wave: selection.wave,
    candidate_id: selection.candidate_id,
    content_id: selection.content_id,
    snapshot_sha256: selection.snapshot_sha256,
    candidate_class: selection.candidate_class,
    changed_paths: [...selection.classification_inputs.changed_paths],
    signals: [...selection.classification_inputs.signals],
    ambiguous: selection.ambiguity_widened,
  });
}

const SELECTION_KEYS = [
  "artifact_kind",
  "schema_version",
  "lifecycle_id",
  "phase",
  "program",
  "wave",
  "candidate_id",
  "content_id",
  "snapshot_sha256",
  "selection_table_version",
  "candidate_class",
  "classification_inputs",
  "ambiguity_widened",
  "profiles",
  "roster",
];

const CANDIDATE_CLASSES = Object.freeze([
  "code",
  "configuration",
  "documentation",
  "ambiguous",
]);

function canonicalClassificationArray(value, label, kind) {
  if (!Array.isArray(value)) error(`${label} must be an array`);
  const normalized = value.map((entry) => {
    if (typeof entry !== "string" || entry.trim() === "") {
      error(`${label} must contain non-blank strings`);
    }
    if (CONTROL_CHARACTER_PATTERN.test(entry)) {
      error(`${label} must not contain control characters`);
    }
    const canonicalEntry = kind === "changed_paths"
      ? posix.normalize(entry)
      : entry.trim().toLowerCase();
    if (
      canonicalEntry !== entry ||
      (kind === "changed_paths" &&
        (canonicalEntry === "." ||
          canonicalEntry.startsWith("/") ||
          canonicalEntry.endsWith("/")))
    ) {
      error(
        `${label} must contain canonical normalized ` +
        `${kind === "changed_paths" ? "paths" : "signals"}`,
      );
    }
    return canonicalEntry;
  });
  const canonical = sortUtf8([...new Set(normalized)]);
  if (normalized.join("\u0000") !== canonical.join("\u0000")) {
    error(
      `${label} must be ${kind === "changed_paths"
        ? "unique and sorted"
        : "unique, lowercase, and sorted"}`,
    );
  }
  return normalized;
}

function parseClassificationInputs(
  value,
  label,
  { allowNested = false, allowEmptyFixDeltaPaths = false } = {},
) {
  if (!isPlainObject(value)) error(`${label} must be an object`);
  const allowedKeys = new Set([
    "changed_paths",
    "signals",
    "candidate_class",
    "ambiguous",
    ...(allowNested ? ["full_candidate", "fix_delta"] : []),
  ]);
  for (const key of Object.keys(value)) {
    if (!allowedKeys.has(key)) {
      error(`${label} contains unknown field "${key}"`);
    }
  }
  for (const key of ["changed_paths", "signals", "candidate_class", "ambiguous"]) {
    if (!Object.hasOwn(value, key)) error(`${label} must contain ${key}`);
  }

  const changedPaths = canonicalClassificationArray(
    value.changed_paths,
    `${label}.changed_paths`,
    "changed_paths",
  );
  const signals = canonicalClassificationArray(
    value.signals,
    `${label}.signals`,
    "signals",
  );
  if (typeof value.candidate_class !== "string" || value.candidate_class.trim() === "") {
    error(`${label}.candidate_class must be a non-blank string`);
  }
  const candidateClass = value.candidate_class;
  if (!CANDIDATE_CLASSES.includes(candidateClass)) {
    error(`${label}.candidate_class "${candidateClass}" is unsupported`);
  }
  if (
    candidateClass === "documentation" &&
    changedPaths.some((path) => !isDocumentationPath(path))
  ) {
    error(
      `${label} candidate_class documentation cannot narrow actual non-documentation paths`,
    );
  }
  if (typeof value.ambiguous !== "boolean") {
    error(`${label}.ambiguous must be boolean`);
  }
  /*
   * A verification selection with an omitted fix delta uses an empty
   * fix_delta sentinel. Rust permits that sentinel to retain the full
   * candidate class and a non-widening ambiguity bit.
   */
  const emptyFixDeltaSentinel =
    allowEmptyFixDeltaPaths &&
    changedPaths.length === 0 &&
    signals.length === 0 &&
    value.ambiguous === false &&
    candidateClass === "ambiguous";
  if (value.ambiguous !== (candidateClass === "ambiguous") && !emptyFixDeltaSentinel) {
    error(`${label} candidate_class and ambiguous disagree`);
  }
  if (
    ["code", "configuration"].includes(candidateClass) &&
    changedPaths.length === 0 &&
    (!allowEmptyFixDeltaPaths || signals.length > 0)
  ) {
    error(`${label} ${candidateClass} classification must contain changed paths`);
  }

  const fullCandidate = Object.hasOwn(value, "full_candidate")
    ? parseClassificationInputs(
        value.full_candidate,
        `${label}.full_candidate`,
      )
    : undefined;
  const fixDelta = Object.hasOwn(value, "fix_delta")
    ? parseClassificationInputs(value.fix_delta, `${label}.fix_delta`, {
        allowEmptyFixDeltaPaths: true,
      })
    : undefined;
  return {
    changed_paths: changedPaths,
    signals,
    candidate_class: candidateClass,
    ambiguous: value.ambiguous,
    ...(fullCandidate ? { full_candidate: fullCandidate } : {}),
    ...(fixDelta ? { fix_delta: fixDelta } : {}),
  };
}

function validateNestedClassificationConsistency(inputs) {
  const nested = [inputs.full_candidate, inputs.fix_delta].filter(Boolean);
  if (nested.length === 0) return;
  const changedPaths = sortUtf8([...new Set(
    nested.flatMap((classification) => classification.changed_paths),
  )]);
  if (inputs.changed_paths.join("\u0000") !== changedPaths.join("\u0000")) {
    error(
      "panel selection classification_inputs changed_paths must equal the union of its " +
      "nested full_candidate and fix_delta paths",
    );
  }
  const signals = sortUtf8([...new Set(
    nested.flatMap((classification) => classification.signals),
  )]);
  if (inputs.signals.join("\u0000") !== signals.join("\u0000")) {
    error(
      "panel selection classification_inputs signals must equal the union of its nested " +
      "full_candidate and fix_delta signals",
    );
  }
  const ambiguous = nested.some((classification) => classification.ambiguous);
  if (inputs.ambiguous !== ambiguous) {
    error(
      "panel selection classification_inputs ambiguous must equal nested classifications",
    );
  }
  const nestedClasses = nested.map((classification) => classification.candidate_class);
  const expectedClass = candidateClassPrecedence(nestedClasses);
  if (inputs.candidate_class !== expectedClass) {
    error(
      "panel selection classification_inputs candidate_class must agree with nested " +
      "classifications",
    );
  }
}

export function validateSelection(selection, table = readSelectionTable()) {
  assertExactKeys(selection, SELECTION_KEYS, "lifecycle selection");
  if (selection.artifact_kind !== LIFECYCLE_SELECTION_ARTIFACT) {
    error("lifecycle selection has an unexpected artifact_kind");
  }
  if (selection.schema_version !== SELECTION_SCHEMA_VERSION) {
    error(`lifecycle selection schema_version must be ${SELECTION_SCHEMA_VERSION}`);
  }
  nonBlank(selection.lifecycle_id, "selection.lifecycle_id");
  if (!["discovery", "verification"].includes(selection.phase)) {
    error("selection.phase must be discovery or verification");
  }
  nonBlank(selection.program, "selection.program");
  nonBlank(selection.wave, "selection.wave");
  safePathPart(selection.candidate_id, "selection.candidate_id");
  nonBlank(selection.content_id, "selection.content_id");
  assertDigest(selection.snapshot_sha256, "selection.snapshot_sha256");
  if (selection.selection_table_version !== SELECTION_TABLE_VERSION) {
    error(
      `selection_table_version must be ${SELECTION_TABLE_VERSION}; found ${selection.selection_table_version}`,
    );
  }
  nonBlank(selection.candidate_class, "selection.candidate_class");
  if (!isPlainObject(selection.classification_inputs)) {
    error("selection.classification_inputs must be an object");
  }
  const classificationInputs = parseClassificationInputs(
    selection.classification_inputs,
    "selection.classification_inputs",
    { allowNested: selection.phase === "verification" },
  );
  if (
    selection.phase === "verification" &&
    (!classificationInputs.full_candidate || !classificationInputs.fix_delta)
  ) {
    error(
      "verification selection classification_inputs must contain both " +
      "full_candidate and fix_delta",
    );
  }
  if (classificationInputs.candidate_class !== selection.candidate_class) {
    error("selection classification candidate_class disagrees with selection");
  }
  if (classificationInputs.ambiguous !== selection.ambiguity_widened) {
    error("selection classification ambiguity disagrees with selection");
  }
  if (typeof selection.ambiguity_widened !== "boolean") {
    error("selection.ambiguity_widened must be boolean");
  }
  if (!isPlainObject(selection.profiles)) error("selection.profiles must be an object");
  if (!["code", "configuration", "documentation", "ambiguous"].includes(selection.candidate_class)) {
    error(`selection.candidate_class "${selection.candidate_class}" is unsupported`);
  }
  validateNestedClassificationConsistency(classificationInputs);
  const roster = validateRoster(selection.roster, table, "selection.roster");
  const canonicalRoster = seatOrder(table).filter((seat) => roster.includes(seat));
  if (roster.join(",") !== canonicalRoster.join(",")) {
    error("selection roster is not in deterministic table order");
  }
  if (roster.length < table.floors[selection.candidate_class]) {
    error(
      `selection roster has ${roster.length} seats but ${selection.candidate_class} requires floor ${table.floors[selection.candidate_class]}`,
    );
  }
  for (const seat of roster) {
    if (!Array.isArray(selection.profiles[seat])) {
      error(`selection.profiles is missing an array for ${seat}`);
    }
    const knownProfiles = new Set(Object.keys(table.seats[seat].profiles));
    for (const profile of selection.profiles[seat]) {
      if (typeof profile !== "string" || !knownProfiles.has(profile)) {
        error(`selection profile ${seat}/${profile} is not defined by the selection table`);
      }
    }
    const expectedProfiles = profilesForSeat(table, seat, {
      changed_paths: selection.classification_inputs.changed_paths,
      signals: selection.classification_inputs.signals,
    });
    for (const profile of expectedProfiles) {
      if (!selection.profiles[seat].includes(profile)) {
        error(`selection profile ${seat}/${profile} is missing for its classification inputs`);
      }
    }
  }
  for (const seat of Object.keys(selection.profiles)) {
    if (!roster.includes(seat)) {
      error(`selection roster/profile mismatch: profiles contains unselected seat ${seat}`);
    }
  }
  validateSelectionAgainstTable(selection, table);
  return selection;
}

export function validateSelectionCandidate(selection, expected) {
  const address = candidateAddress(expected);
  for (const key of ["program", "wave", "candidate_id", "content_id", "snapshot_sha256"]) {
    if (selection[key] !== address[key]) {
      error(
        `selection ${key} "${selection[key]}" disagrees with candidate "${address[key]}"`,
      );
    }
  }
  return true;
}

const CURRENT_CANDIDATE_OPTIONAL_KEYS = [
  "candidate_class",
  "changed_paths",
  "signals",
  "ambiguous",
];

export function validateCandidateAgainstSelection(
  selection,
  currentCandidate,
  table = readSelectionTable(),
) {
  validateSelection(selection, table);
  if (!isPlainObject(currentCandidate)) {
    error("current candidate must be a JSON object");
  }
  const candidateKeys = Object.keys(currentCandidate);
  const allowedKeys = new Set([
    "program",
    "wave",
    "candidate_id",
    "content_id",
    "snapshot_sha256",
    ...CURRENT_CANDIDATE_OPTIONAL_KEYS,
  ]);
  const unknown = candidateKeys.filter((key) => !allowedKeys.has(key));
  if (unknown.length > 0) {
    error(
      `current candidate contains unknown field(s): ${unknown.join(", ")}`,
    );
  }
  validateSelectionCandidate(selection, currentCandidate);
  const hasClassification = CURRENT_CANDIDATE_OPTIONAL_KEYS.some((key) =>
    Object.hasOwn(currentCandidate, key),
  );
  if (!hasClassification) return true;
  const actual = candidateInputs(currentCandidate);
  const actualClass = actual.candidate_class ?? inferCandidateClass(actual);
  const expected =
    selection.phase === "verification"
      ? selection.classification_inputs.full_candidate
      : selection.classification_inputs;
  if (
    Object.hasOwn(currentCandidate, "changed_paths") &&
    actual.changed_paths.join("\u0000") !== expected.changed_paths.join("\u0000")
  ) {
    error(
      "current candidate changed_paths disagree with the exact lifecycle selection",
    );
  }
  if (
    Object.hasOwn(currentCandidate, "signals") &&
    actual.signals.join("\u0000") !== expected.signals.join("\u0000")
  ) {
    error(
      "current candidate signals disagree with the exact lifecycle selection",
    );
  }
  if (
    Object.hasOwn(currentCandidate, "candidate_class") &&
    actualClass !== expected.candidate_class
  ) {
    error(
      "current candidate candidate_class disagrees with the exact lifecycle selection",
    );
  }
  if (
    Object.hasOwn(currentCandidate, "ambiguous") &&
    actual.ambiguous !== expected.ambiguous
  ) {
    error(
      "current candidate ambiguity disagrees with the exact lifecycle selection",
    );
  }
  return true;
}

export const validateCurrentCandidate = validateCandidateAgainstSelection;

export function createSelection(input, options = {}) {
  const table = options.table ?? readSelectionTable(options.table_path);
  const address = candidateAddress(input);
  const lifecycleId = nonBlank(
    input.lifecycle_id ?? input.lifecycleId ?? options.lifecycle_id ?? options.lifecycleId,
    "lifecycle_id",
  );
  const phase = input.phase ?? options.phase ?? "discovery";
  if (!["discovery", "verification"].includes(phase)) {
    error("phase must be discovery or verification");
  }
  if (
    phase === "discovery" &&
    (input.previous_selection ??
      input.previousSelection ??
      options.previous_selection ??
      options.previousSelection ??
      input.fix_delta ??
      input.fixDelta ??
      input.delta)
  ) {
    error("prior selection and fix delta are verification-only selection inputs");
  }
  const plan = phase === "verification" ||
      input.fix_delta || input.fixDelta || input.delta ||
      input.full_candidate || input.fullCandidate
    ? selectLifecycleRoster(input, { table })
    : selectRoster(input, { table });
  const selectionPlan = plan.full
    ? {
        ...plan.full,
        candidate_class: candidateClassPrecedence([
          plan.full.candidate_class,
          plan.delta.candidate_class,
        ]),
        classification_inputs: {
          ...plan.full.classification_inputs,
          changed_paths: sortUtf8([...new Set([
            ...plan.full.classification_inputs.changed_paths,
            ...plan.delta.classification_inputs.changed_paths,
          ])]),
          signals: sortUtf8([...new Set([
            ...plan.full.classification_inputs.signals,
            ...plan.delta.classification_inputs.signals,
          ])]),
          candidate_class: candidateClassPrecedence([
            plan.full.candidate_class,
            plan.delta.candidate_class,
          ]),
          ambiguous:
            plan.full.ambiguity_widened || plan.delta.ambiguity_widened,
          full_candidate: plan.full.classification_inputs,
          fix_delta: plan.delta.classification_inputs,
        },
        ambiguity_widened:
          plan.full.ambiguity_widened || plan.delta.ambiguity_widened,
        roster: plan.roster,
        profiles: plan.profiles,
        triggered_optional: [
          ...new Set([
            ...plan.full.triggered_optional,
            ...plan.delta.triggered_optional,
          ]),
        ],
        floor_filled: [
          ...new Set([
            ...plan.full.floor_filled,
            ...plan.delta.floor_filled,
          ]),
        ],
      }
    : plan;
  const previousSelectionInput =
    input.previous_selection ??
    input.previousSelection ??
    options.previous_selection ??
    options.previousSelection;
  const previousSelection = typeof previousSelectionInput === "string"
    ? readSelection(previousSelectionInput, { table })
    : previousSelectionInput;
  if (previousSelection) {
    validateSelection(previousSelection, table);
    if (previousSelection.lifecycle_id !== lifecycleId) {
      error(
        `previous selection lifecycle "${previousSelection.lifecycle_id}" disagrees with "${lifecycleId}"`,
      );
    }
  }
  const previousRoster =
    input.previous_roster ??
    input.previousRoster ??
    options.previous_roster ??
    options.previousRoster ??
    previousSelection?.roster;
  const previousProfiles =
    input.previous_profiles ??
    options.previous_profiles ??
    previousSelection?.profiles ??
    {};
  const roster = previousRoster
    ? unionRosters([previousRoster, selectionPlan.roster], table)
    : selectionPlan.roster;
  if (previousRoster) validateMonotonicRoster(previousRoster, roster, table);
  const profiles = Object.fromEntries(
    roster.map((seat) => [
      seat,
      [...new Set([
        ...(selectionPlan.profiles[seat] ?? []),
        ...(previousProfiles[seat] ?? []),
      ])].sort(),
    ]),
  );
  const selection = {
    artifact_kind: LIFECYCLE_SELECTION_ARTIFACT,
    schema_version: SELECTION_SCHEMA_VERSION,
    lifecycle_id: lifecycleId,
    phase,
    ...address,
    selection_table_version: SELECTION_TABLE_VERSION,
    candidate_class: selectionPlan.candidate_class,
    classification_inputs: selectionPlan.classification_inputs,
    ambiguity_widened: selectionPlan.ambiguity_widened,
    profiles,
    roster,
  };
  validateSelection(selection, table);
  const root = resolve(options.root ?? process.cwd());
  const path = options.path ??
    selectionPath(
      root,
      lifecycleId,
      address.candidate_id,
      address.snapshot_sha256,
      phase,
    );
  const result = writeCreateOrCompare(path, selection);
  return { selection, path: result.path, created: result.created, plan: selectionPlan };
}

export function readSelection(path, options = {}) {
  const selection = readJson(path, "lifecycle selection");
  return validateSelection(selection, options.table ?? readSelectionTable(options.table_path));
}

function selectionSummary(selection, table) {
  const valid = validateSelection(selection, table);
  return {
    lifecycle_id: valid.lifecycle_id,
    phase: valid.phase,
    program: valid.program,
    wave: valid.wave,
    candidate_id: valid.candidate_id,
    content_id: valid.content_id,
    snapshot_sha256: valid.snapshot_sha256,
    selection_schema_version: valid.schema_version,
    selection_table_version: valid.selection_table_version,
    roster: [...valid.roster],
    profiles: valid.profiles,
  };
}

export function createDiscoveryRequest(input, options = {}) {
  const table = options.table ?? readSelectionTable(options.table_path);
  const selection = input.selection ?? readSelection(input.selection_path, { table });
  validateSelection(selection, table);
  if (selection.phase !== "discovery") {
    error("a discovery request requires a discovery selection");
  }
  const candidate = input.candidate ?? input;
  validateSelectionCandidate(selection, candidate);
  const context = input.context ?? input.relevant_context ?? {};
  const validationEvidence =
    input.validation_evidence ?? input.evidence ?? [];
  if (!Array.isArray(validationEvidence)) {
    error("validation_evidence must be an array");
  }
  const seats = selection.roster.map((seat) => ({
    seat,
    profiles: selection.profiles[seat],
    focus: table.seats[seat].focus,
    obligation:
      "Review the full candidate comprehensively and report every reasonably discoverable actionable finding.",
  }));
  const request = {
    artifact_kind: DISCOVERY_REQUEST_ARTIFACT,
    schema_version: SELECTION_SCHEMA_VERSION,
    lifecycle_id: selection.lifecycle_id,
    phase: "discovery",
    selection: selectionSummary(selection, table),
    candidate: {
      program: selection.program,
      wave: selection.wave,
      candidate_id: selection.candidate_id,
      content_id: selection.content_id,
      snapshot_sha256: selection.snapshot_sha256,
    },
    full_candidate: true,
    comprehensive: true,
    instruction:
      "This first review is comprehensive. Spend the effort now, report every reasonably discoverable actionable finding, and do not save observations for later rounds.",
    context,
    validation_evidence: validationEvidence,
    seats,
  };
  return sortedObject(request);
}

export function writeDiscoveryRequest(path, input, options = {}) {
  return writeCreateOrCompare(path, createDiscoveryRequest(input, options));
}

function resultEntries(results) {
  if (Array.isArray(results)) return results.map((result) => [result?.seat, result]);
  if (isPlainObject(results)) {
    return Object.entries(results).map(([seat, result]) => [
      result?.seat ?? seat,
      { ...(isPlainObject(result) ? result : {}), seat: result?.seat ?? seat },
    ]);
  }
  error("discovery results must be an array or object keyed by seat");
}

function normalizeSeverity(value, label) {
  if (!SEVERITIES.includes(value)) error(`${label} must be one of ${SEVERITIES.join(", ")}`);
  return value;
}

function verdictSeverity(value, label) {
  const aliases = {
    critical: "BLOCKER",
    blocker: "BLOCKER",
    high: "MAJOR",
    major: "MAJOR",
    medium: "MINOR",
    minor: "MINOR",
    low: "NIT",
    nit: "NIT",
  };
  if (typeof value !== "string") {
    error(`${label} must be a severity string`);
  }
  const normalized = aliases[value.toLowerCase()];
  if (!normalized) {
    error(`${label} must be one of critical, high, medium, low, BLOCKER, MAJOR, MINOR, NIT`);
  }
  return normalized;
}

function verdictEntries(input) {
  const value = input?.verdicts ?? input?.results ?? input;
  if (Array.isArray(value)) {
    return value.map((verdict) => [verdict?.engineer ?? verdict?.seat, verdict]);
  }
  if (isPlainObject(value)) {
    return Object.entries(value).map(([seat, verdict]) => [
      verdict?.engineer ?? verdict?.seat ?? seat,
      verdict,
    ]);
  }
  error("verdicts must be an array or object keyed by seat");
}

function validateActualVerdict(verdict, label = "verdict") {
  if (!isPlainObject(verdict)) error(`${label} must be a JSON object`);
  const seat = verdict.engineer ?? verdict.seat;
  nonBlank(seat, `${label}.engineer`);
  if (typeof verdict.signoff !== "boolean") {
    error(`${label}.signoff must be a boolean`);
  }
  if (typeof verdict.summary !== "string" || verdict.summary.trim() === "") {
    error(`${label}.summary must be a non-blank string`);
  }
  if (!Array.isArray(verdict.recommendations)) {
    error(`${label}.recommendations must be an array`);
  }
  if (verdict.signoff !== (verdict.recommendations.length === 0)) {
    error(`${label}.signoff must be true if and only if recommendations is empty`);
  }
  return seat;
}

function recommendationFinding(seat, recommendation, index) {
  const label = `verdict ${seat} recommendation ${index + 1}`;
  let value = recommendation;
  if (typeof recommendation === "string") {
    nonBlank(recommendation, label);
    value = {
      severity: "MAJOR",
      raw_text: recommendation,
      impact: "Impact supplied by the reviewer.",
      recommendation,
    };
  } else if (!isPlainObject(recommendation)) {
    error(`${label} must be a string or object`);
  }
  const sourceOrdinal = value.source_ordinal ?? value.ordinal ?? index + 1;
  if (!Number.isInteger(sourceOrdinal) || sourceOrdinal < 1) {
    error(`${label}.source_ordinal must be a positive integer`);
  }
  const severity = verdictSeverity(value.severity ?? "MAJOR", `${label}.severity`);
  const where = value.where;
  const what = value.what ?? value.description;
  const why = value.why ?? value.impact;
  const fix = value.fix ?? value.recommendation;
  const rawText = value.raw_text ??
    (typeof recommendation === "string"
      ? recommendation
      : [where, what, why, fix].filter((part) => typeof part === "string" && part !== "").join(": "));
  nonBlank(rawText, `${label}.raw_text`);
  const impact = why ?? "Impact supplied by the reviewer.";
  const recommendationText = fix ?? what ?? rawText;
  nonBlank(impact, `${label}.impact`);
  nonBlank(recommendationText, `${label}.recommendation`);
  const rendered = {
    source_id: value.source_id ?? `${seat}:${sourceOrdinal}`,
    seat,
    source_ordinal: sourceOrdinal,
    raw_text: rawText,
    attribution: value.attribution ?? seat,
    severity,
    impact,
    recommendation: recommendationText,
  };
  nonBlank(rendered.source_id, `${label}.source_id`);
  nonBlank(rendered.attribution, `${label}.attribution`);
  return rendered;
}

export function adaptDiscoveryVerdict(verdict, options = {}) {
  const seat = validateActualVerdict(verdict, "discovery verdict");
  const expectedSeat = options.seat ?? seat;
  if (seat !== expectedSeat) {
    error(`discovery verdict engineer "${seat}" disagrees with selected seat "${expectedSeat}"`);
  }
  return {
    seat,
    complete: true,
    findings: verdict.recommendations.map((recommendation, index) =>
      recommendationFinding(seat, recommendation, index),
    ),
  };
}

export function adaptDiscoveryResults(input, options = {}) {
  const adapted = verdictEntries(input).map(([seat, verdict]) =>
    adaptDiscoveryVerdict(verdict, { ...options, seat }),
  );
  return adapted.sort((left, right) =>
    (options.selection?.roster?.indexOf(left.seat) ?? Number.MAX_SAFE_INTEGER) -
      (options.selection?.roster?.indexOf(right.seat) ?? Number.MAX_SAFE_INTEGER) ||
    String(left.seat).localeCompare(String(right.seat)),
  );
}

export function adaptVerificationVerdict(verdict, options = {}) {
  const seat = validateActualVerdict(verdict, "verification verdict");
  const expectedSeat = options.seat ?? seat;
  if (seat !== expectedSeat) {
    error(`verification verdict engineer "${seat}" disagrees with selected seat "${expectedSeat}"`);
  }
  const statuses = verdict.verified_issue_statuses ??
    verdict.issue_statuses ??
    verdict.verification_statuses;
  if (options.issue_ids && statuses === undefined) {
    error(`verification verdict ${seat} must contain exact per-issue verification status`);
  }
  return {
    seat,
    complete: true,
    summary: verdict.summary,
    signoff: verdict.signoff,
    verified_issue_statuses: statuses ?? {},
    blocking_recommendations: verdict.recommendations,
    recommendations: verdict.recommendations,
    late_findings: verdict.late_findings ?? [],
  };
}

export function adaptVerificationResults(input, options = {}) {
  const adapted = verdictEntries(input).map(([seat, verdict]) =>
    adaptVerificationVerdict(verdict, { ...options, seat }),
  );
  return adapted.sort((left, right) =>
    (options.selection?.roster?.indexOf(left.seat) ?? Number.MAX_SAFE_INTEGER) -
      (options.selection?.roster?.indexOf(right.seat) ?? Number.MAX_SAFE_INTEGER) ||
    String(left.seat).localeCompare(String(right.seat)),
  );
}

const ADAPTED_VERIFICATION_KEYS = [
  "artifact_kind",
  "schema_version",
  "phase",
  "lifecycle_id",
  "selection_sha256",
  "current_candidate",
  "discovery_ledger_sha256",
  "results",
];

export function createVerificationResultArtifact(input, options = {}) {
  const table = options.table ?? readSelectionTable(options.table_path);
  const selection = input.selection ??
    (input.selection_path ? readSelection(input.selection_path, { table }) : undefined);
  if (!selection) error("adapted verification requires the current selection");
  validateSelection(selection, table);
  if (selection.phase !== "verification") {
    error("adapted verification requires a verification selection");
  }
  const ledgerBytes = input.ledger_bytes ?? input.discovery_ledger_bytes;
  if (typeof ledgerBytes !== "string" || ledgerBytes.length === 0) {
    error("adapted verification requires the exact immutable discovery ledger bytes");
  }
  const ledger = input.ledger ?? input.discovery_ledger;
  if (!ledger) error("adapted verification requires the immutable discovery ledger");
  validateLedger(ledger);
  try {
    if (stableStringify(JSON.parse(ledgerBytes)) !== stableStringify(ledger)) {
      error("adapted verification ledger object disagrees with the exact ledger bytes");
    }
  } catch (cause) {
    error(`adapted verification ledger bytes are not valid JSON: ${cause.message}`);
  }
  const currentCandidate = input.current_candidate ??
    input.currentCandidate ??
    input.candidate;
  if (!currentCandidate) {
    error("adapted verification requires the current candidate");
  }
  validateSelectionCandidate(selection, currentCandidate);
  const results = validateVerificationResults(
    selection,
    input.results ?? input.verification_results ?? input.verdicts,
    { table, ledger },
  );
  return sortedObject({
    artifact_kind: VERIFICATION_ARTIFACT,
    schema_version: SELECTION_SCHEMA_VERSION,
    phase: "verification",
    lifecycle_id: selection.lifecycle_id,
    selection_sha256: sha256(
      input.selection_bytes ?? stableStringify(selection),
    ),
    current_candidate: candidateAddress(currentCandidate),
    discovery_ledger_sha256: sha256(ledgerBytes),
    results,
  });
}

export function validateVerificationResultArtifact(
  artifact,
  options = {},
) {
  assertExactKeys(artifact, ADAPTED_VERIFICATION_KEYS, "adapted verification result");
  if (artifact.artifact_kind !== VERIFICATION_ARTIFACT) {
    error("adapted verification result has an unexpected artifact_kind");
  }
  if (artifact.schema_version !== SELECTION_SCHEMA_VERSION) {
    error("adapted verification result schema_version is unsupported");
  }
  if (artifact.phase !== "verification") {
    error("adapted verification result phase must be verification");
  }
  const selection = options.selection;
  if (!selection) error("adapted verification validation requires the current selection");
  const table = options.table ?? readSelectionTable(options.table_path);
  validateSelection(selection, table);
  if (artifact.lifecycle_id !== selection.lifecycle_id) {
    error("adapted verification result lifecycle_id disagrees with selection");
  }
  const selectionBytes = options.selection_bytes;
  if (typeof selectionBytes === "string") {
    if (artifact.selection_sha256 !== sha256(selectionBytes)) {
      error("adapted verification result is not bound to the exact selection bytes");
    }
  } else if (artifact.selection_sha256 !== sha256(selection)) {
    error("adapted verification result selection digest does not match selection");
  }
  const ledger = options.ledger;
  if (!ledger) error("adapted verification validation requires the immutable discovery ledger");
  validateLedger(ledger);
  if (typeof options.ledger_bytes !== "string" || options.ledger_bytes.length === 0) {
    error("adapted verification validation requires exact ledger bytes");
  }
  if (artifact.discovery_ledger_sha256 !== sha256(options.ledger_bytes)) {
    error("adapted verification result is not bound to the exact ledger bytes");
  }
  validateSelectionCandidate(selection, artifact.current_candidate);
  validateMonotonicRoster(ledger.roster, selection.roster, table);
  return validateVerificationResults(selection, artifact.results, {
    table,
    ledger,
  });
}

export const verdictToDiscoveryResult = adaptDiscoveryVerdict;
export const verdictsToDiscoveryResults = adaptDiscoveryResults;
export const verdictToVerificationResult = adaptVerificationVerdict;
export const verdictsToVerificationResults = adaptVerificationResults;

function normalizeSourceFinding(seat, finding, index) {
  if (!isPlainObject(finding)) error(`finding ${seat}:${index + 1} must be an object`);
  const sourceOrdinal = finding.source_ordinal ?? finding.ordinal ?? index + 1;
  if (!Number.isInteger(sourceOrdinal) || sourceOrdinal < 1) {
    error(`finding ${seat}:${index + 1} source_ordinal must be a positive integer`);
  }
  const sourceId = finding.source_id ?? `${seat}:${sourceOrdinal}`;
  nonBlank(sourceId, `finding ${seat}:${index + 1} source_id`);
  const rawText =
    finding.raw_text ??
    finding.text ??
    finding.recommendation ??
    finding.description;
  nonBlank(rawText, `finding ${seat}:${index + 1} raw_text`);
  const attribution = finding.attribution ?? finding.raw_attribution ?? seat;
  nonBlank(attribution, `finding ${seat}:${index + 1} attribution`);
  const impact = finding.impact ?? "Impact supplied by the reviewer.";
  const recommendation = finding.recommendation ?? finding.fix ?? rawText;
  nonBlank(impact, `finding ${seat}:${index + 1} impact`);
  nonBlank(recommendation, `finding ${seat}:${index + 1} recommendation`);
  return {
    source_id: sourceId,
    seat,
    source_ordinal: sourceOrdinal,
    raw_text: rawText,
    attribution,
    severity: normalizeSeverity(finding.severity, `finding ${seat}:${index + 1} severity`),
    impact,
    recommendation,
    ...(finding.migration_assigned_severity === true
      ? { migration_assigned_severity: true }
      : {}),
  };
}

export function validateDiscoveryResults(selection, results, options = {}) {
  const table = options.table ?? readSelectionTable(options.table_path);
  validateSelection(selection, table);
  const rawEntries = resultEntries(results);
  const actualVerdicts = rawEntries.length > 0 &&
    rawEntries.every(([, result]) =>
      isPlainObject(result) &&
      Object.hasOwn(result, "engineer") &&
      !Object.hasOwn(result, "complete"),
    );
  const adapted = actualVerdicts
    ? Object.fromEntries(adaptDiscoveryResults(results).map((result) => [result.seat, result]))
    : results;
  if (
    rawEntries.some(([, result]) =>
      isPlainObject(result) &&
      Object.hasOwn(result, "engineer") &&
      !Object.hasOwn(result, "complete"),
    ) !== actualVerdicts
  ) {
    error("discovery results must not mix actual verdict JSON with discovery result shapes");
  }
  const expected = new Set(selection.roster);
  const seenSeats = new Set();
  const sources = [];
  for (const [seat, result] of resultEntries(adapted)) {
    nonBlank(seat, "discovery result seat");
    if (!expected.has(seat)) error(`discovery result for unselected seat "${seat}"`);
    if (seenSeats.has(seat)) error(`duplicate discovery result for seat "${seat}"`);
    seenSeats.add(seat);
    if (!isPlainObject(result) || result.complete !== true) {
      error(`discovery result for ${seat} must explicitly set complete: true`);
    }
    if (!Array.isArray(result.findings)) {
      error(`discovery result for ${seat} must contain a findings array`);
    }
    const normalized = result.findings.map((finding, index) =>
      normalizeSourceFinding(seat, finding, index),
    );
    normalized.sort((left, right) => left.source_ordinal - right.source_ordinal);
    sources.push(...normalized);
  }
  for (const seat of selection.roster) {
    if (!seenSeats.has(seat)) {
      error(
        `missing complete discovery result for selected seat "${seat}"; an absent result is not zero findings`,
      );
    }
  }
  const sourceIds = new Set();
  const sourceOrdinals = new Set();
  for (const source of sources) {
    if (sourceIds.has(source.source_id)) {
      error(`duplicate source finding id "${source.source_id}"`);
    }
    const ordinalKey = `${source.seat}:${source.source_ordinal}`;
    if (sourceOrdinals.has(ordinalKey)) {
      error(`duplicate source ordinal "${ordinalKey}"`);
    }
    sourceIds.add(source.source_id);
    sourceOrdinals.add(ordinalKey);
  }
  sources.sort((left, right) => {
    const seatOrderValue = selection.roster.indexOf(left.seat) - selection.roster.indexOf(right.seat);
    return seatOrderValue || left.source_ordinal - right.source_ordinal ||
      left.source_id.localeCompare(right.source_id);
  });
  return sources;
}

function groupSourceIds(group) {
  const ids = group.source_finding_ids ?? group.source_ids ?? group.sources;
  if (!Array.isArray(ids) || ids.some((id) => typeof id !== "string" || id.trim() === "")) {
    error("each deduplication group must contain source_finding_ids");
  }
  return [...ids];
}

function maxSeverity(sources) {
  return sources
    .map((source) => source.severity)
    .sort((left, right) => SEVERITY_RANK[right] - SEVERITY_RANK[left])[0];
}

export function mergeDiscoveryLedger(input, options = {}) {
  const table = options.table ?? readSelectionTable(options.table_path);
  const selection = input.selection ?? readSelection(input.selection_path, { table });
  validateSelection(selection, table);
  const sources = validateDiscoveryResults(selection, input.results ?? input.discovery_results, { table });
  if (!Array.isArray(input.groups ?? input.dedup_groups)) {
    error("orchestrator-supplied dedup_groups are required");
  }
  const groups = input.groups ?? input.dedup_groups;
  const sourceById = new Map(sources.map((source) => [source.source_id, source]));
  const mapped = new Set();
  const issues = [];
  for (const [index, group] of groups.entries()) {
    if (!isPlainObject(group)) error(`deduplication group ${index + 1} must be an object`);
    const sourceIds = groupSourceIds(group);
    if (sourceIds.length === 0) error(`deduplication group ${index + 1} is empty`);
    for (const sourceId of sourceIds) {
      if (!sourceById.has(sourceId)) {
        error(`deduplication group ${index + 1} references unknown source "${sourceId}"`);
      }
      if (mapped.has(sourceId)) {
        error(`source finding "${sourceId}" maps to more than one ledger issue`);
      }
      mapped.add(sourceId);
    }
    const groupId = group.id ?? group.ledger_id ?? group.issue_id;
    const id = groupId ?? `R${index + 1}`;
    if (!/^R[1-9][0-9]*$/.test(id)) {
      error(`ledger issue id "${id}" must use the stable R<n> form`);
    }
    const groupedSources = sourceIds.map((sourceId) => sourceById.get(sourceId));
    const description = group.description ?? group.what ?? groupedSources[0].raw_text;
    const impact = group.impact ?? groupedSources[0].impact;
    const recommendation =
      group.recommendation ?? group.fix ?? groupedSources[0].recommendation;
    nonBlank(description, `${id}.description`);
    nonBlank(impact, `${id}.impact`);
    nonBlank(recommendation, `${id}.recommendation`);
    const derivedSeverity = maxSeverity(groupedSources);
    const suppliedSeverity = group.severity === undefined
      ? derivedSeverity
      : normalizeSeverity(group.severity, `${id}.severity`);
    if (suppliedSeverity !== derivedSeverity) {
      error(
        `${id}.severity must be the maximum source severity ${derivedSeverity}; ` +
        `received ${suppliedSeverity}`,
      );
    }
    if (group.late !== undefined && typeof group.late !== "boolean") {
      error(`${id}.late must be boolean`);
    }
    issues.push({
      id,
      description,
      severity: derivedSeverity,
      impact,
      recommendation,
      source_finding_ids: sourceIds,
      late: group.late === true,
    });
  }
  if (mapped.size !== sources.length) {
    const missing = sources
      .map((source) => source.source_id)
      .filter((sourceId) => !mapped.has(sourceId));
    error(`source-to-ledger mapping is incomplete; unmapped sources: ${missing.join(", ")}`);
  }
  const ids = issues.map((issue) => issue.id);
  const expectedIds = issues.map((_, index) => `R${index + 1}`);
  if (
    new Set(ids).size !== ids.length ||
    ids.some((id, index) => id !== expectedIds[index])
  ) {
    error(
      `ledger issue ids must be unique and contiguous in orchestrator order: expected ${expectedIds.join(", ")}`,
    );
  }
  const ledger = {
    artifact_kind: LEDGER_ARTIFACT,
    schema_version: SELECTION_SCHEMA_VERSION,
    lifecycle_id: selection.lifecycle_id,
    selection_schema_version: selection.schema_version,
    selection_table_version: selection.selection_table_version,
    program: selection.program,
    wave: selection.wave,
    candidate_id: selection.candidate_id,
    content_id: selection.content_id,
    snapshot_sha256: selection.snapshot_sha256,
    roster: [...selection.roster],
    sources,
    issues,
    complete: true,
  };
  return sortedObject(ledger);
}

export function writeLedger(path, ledger) {
  return writeCreateOrCompare(path, ledger);
}

export function createResponseTemplate(ledger) {
  validateLedger(ledger);
  return sortedObject({
    artifact_kind: RESPONSE_ARTIFACT,
    schema_version: SELECTION_SCHEMA_VERSION,
    selection_schema_version: ledger.selection_schema_version,
    selection_table_version: ledger.selection_table_version,
    lifecycle_id: ledger.lifecycle_id,
    program: ledger.program,
    wave: ledger.wave,
    candidate_id: ledger.candidate_id,
    content_id: ledger.content_id,
    snapshot_sha256: ledger.snapshot_sha256,
    roster: [...ledger.roster],
    responses: ledger.issues.map((issue) => ({
      issue_id: issue.id,
      disposition: null,
      changed_surface: [],
      justification: "",
      evidence: "",
      verified_factual_status: null,
    })),
  });
}

export function writeResponseTemplate(path, ledger) {
  return writeCreateOrCompare(path, createResponseTemplate(ledger));
}

export function validateLedger(ledger, options = {}) {
  if (!isPlainObject(ledger)) error("ledger must be an object");
  assertExactKeys(
    ledger,
    [
      "artifact_kind",
      "schema_version",
      "lifecycle_id",
      "selection_schema_version",
      "selection_table_version",
      "program",
      "wave",
      "candidate_id",
      "content_id",
      "snapshot_sha256",
      "roster",
      "sources",
      "issues",
      "complete",
    ],
    "ledger",
  );
  if (ledger.artifact_kind !== LEDGER_ARTIFACT) {
    error("ledger has an unexpected artifact_kind");
  }
  if (ledger.schema_version !== SELECTION_SCHEMA_VERSION) {
    error("ledger schema_version is unsupported");
  }
  if (ledger.selection_schema_version !== SELECTION_SCHEMA_VERSION) {
    error("ledger selection_schema_version is unsupported");
  }
  if (ledger.selection_table_version !== SELECTION_TABLE_VERSION) {
    error("ledger selection_table_version is unsupported");
  }
  safePathPart(ledger.lifecycle_id, "ledger.lifecycle_id");
  nonBlank(ledger.program, "ledger.program");
  nonBlank(ledger.wave, "ledger.wave");
  safePathPart(ledger.candidate_id, "ledger.candidate_id");
  nonBlank(ledger.content_id, "ledger.content_id");
  assertDigest(ledger.snapshot_sha256, "ledger.snapshot_sha256");
  if (ledger.complete !== true) error("ledger.complete must be true");
  const table = options.table ?? readSelectionTable(options.table_path);
  validateRoster(ledger.roster, table, "ledger.roster");
  if (!Array.isArray(ledger.sources) || !Array.isArray(ledger.issues)) {
    error("ledger sources and issues must be arrays");
  }
  const sourceIds = new Set();
  for (const source of ledger.sources) {
    const allowedSourceKeys = [
      "source_id",
      "seat",
      "source_ordinal",
      "raw_text",
      "attribution",
      "severity",
      "impact",
      "recommendation",
      "raw_attribution",
      "migration_assigned_severity",
    ];
    assertExactKeys(
      source,
      allowedSourceKeys.filter((key) =>
        Object.hasOwn(source, key) ||
        !["raw_attribution", "migration_assigned_severity"].includes(key),
      ),
      "ledger source",
    );
    nonBlank(source.source_id, "ledger source_id");
    nonBlank(source.seat, "ledger source seat");
    if (!Number.isInteger(source.source_ordinal) || source.source_ordinal < 1) {
      error(`ledger source ${source.source_id} source_ordinal must be a positive integer`);
    }
    nonBlank(source.raw_text, `ledger source ${source.source_id}.raw_text`);
    nonBlank(source.attribution, `ledger source ${source.source_id}.attribution`);
    normalizeSeverity(source.severity, `ledger source ${source.source_id}.severity`);
    nonBlank(source.impact, `ledger source ${source.source_id}.impact`);
    nonBlank(source.recommendation, `ledger source ${source.source_id}.recommendation`);
    if (
      source.raw_attribution !== undefined &&
      (typeof source.raw_attribution !== "string" || source.raw_attribution.trim() === "")
    ) {
      error(`ledger source ${source.source_id}.raw_attribution must be non-blank`);
    }
    if (
      source.migration_assigned_severity !== undefined &&
      typeof source.migration_assigned_severity !== "boolean"
    ) {
      error(`ledger source ${source.source_id}.migration_assigned_severity must be boolean`);
    }
    if (sourceIds.has(source.source_id)) error(`ledger repeats source ${source.source_id}`);
    sourceIds.add(source.source_id);
  }
  const issueIds = new Set();
  const mapped = new Set();
  for (const [index, issue] of ledger.issues.entries()) {
    assertExactKeys(
      issue,
      ["id", "description", "severity", "impact", "recommendation", "source_finding_ids", "late"],
      `ledger issue ${index + 1}`,
    );
    if (issue.id !== `R${index + 1}`) error("ledger issue identifiers are not stable and contiguous");
    if (issueIds.has(issue.id)) error(`ledger repeats issue ${issue.id}`);
    issueIds.add(issue.id);
    nonBlank(issue.description, `${issue.id}.description`);
    normalizeSeverity(issue.severity, `${issue.id}.severity`);
    nonBlank(issue.impact, `${issue.id}.impact`);
    nonBlank(issue.recommendation, `${issue.id}.recommendation`);
    if (typeof issue.late !== "boolean") error(`${issue.id}.late must be boolean`);
    if (!Array.isArray(issue.source_finding_ids) || issue.source_finding_ids.length === 0) {
      error(`${issue.id} must map at least one source finding`);
    }
    if (new Set(issue.source_finding_ids).size !== issue.source_finding_ids.length) {
      error(`${issue.id} repeats a source finding id`);
    }
    const issueSources = [];
    for (const sourceId of issue.source_finding_ids) {
      if (!sourceIds.has(sourceId)) error(`${issue.id} references unknown source ${sourceId}`);
      if (mapped.has(sourceId)) error(`source ${sourceId} maps more than once`);
      mapped.add(sourceId);
      issueSources.push(ledger.sources.find((source) => source.source_id === sourceId));
    }
    const derivedSeverity = maxSeverity(issueSources);
    if (issue.severity !== derivedSeverity) {
      error(
        `${issue.id}.severity must be the maximum source severity ${derivedSeverity}; ` +
        `received ${issue.severity}`,
      );
    }
  }
  if (mapped.size !== sourceIds.size) error("ledger does not map every source finding exactly once");
  if (options.selection) {
    for (const key of ["lifecycle_id", "program", "wave"]) {
      if (options.selection[key] !== ledger[key]) {
        error(`ledger and selection ${key} disagree`);
      }
    }
    validateMonotonicRoster(ledger.roster, options.selection.roster, table);
  }
  return true;
}

function responseEntries(responses) {
  if (isPlainObject(responses) && Array.isArray(responses.responses)) {
    return responses.responses.map((response) => [response?.issue_id, response]);
  }
  if (Array.isArray(responses)) return responses.map((response) => [response?.issue_id, response]);
  if (isPlainObject(responses)) {
    return Object.entries(responses).map(([issueId, response]) => [
      response?.issue_id ?? issueId,
      { ...(isPlainObject(response) ? response : {}), issue_id: response?.issue_id ?? issueId },
    ]);
  }
  error("responses must be an array or object keyed by issue id");
}

export function validateAcceptance(acceptance, label = "acceptance") {
  assertExactKeys(acceptance, ["accepter", "capacity", "justification"], label);
  for (const key of ["accepter", "capacity", "justification"]) {
    if (typeof acceptance[key] !== "string") error(`${label}.${key} must be a string`);
  }
  if (acceptance.accepter.trim() === "") error(`${label}.accepter must not be blank`);
  if (acceptance.justification.trim() === "") error(`${label}.justification must not be blank`);
  if (!["repository maintainer", "merge owner"].includes(acceptance.capacity)) {
    error(`${label}.capacity must be repository maintainer or merge owner`);
  }
  return {
    accepter: acceptance.accepter,
    capacity: acceptance.capacity,
    justification: acceptance.justification,
  };
}

function changedSurface(response, label) {
  const value = response.changed_surface ?? response.changed_paths ?? response.surface;
  if (value === undefined) return [];
  const paths = Array.isArray(value) ? value : [value];
  if (paths.some((path) => typeof path !== "string" || path.trim() === "")) {
    error(`${label}.changed_surface must contain non-blank path strings`);
  }
  for (const path of paths) {
    utf8Bytes(path, `${label}.changed_surface path`);
    if (CONTROL_CHARACTER_PATTERN.test(path)) {
      error(`${label}.changed_surface must not contain control characters`);
    }
  }
  return sortUtf8([...new Set(paths)]);
}

export function validateResponseEnvelope(ledger, envelope) {
  assertExactKeys(
    envelope,
    [
      "artifact_kind",
      "schema_version",
      "selection_schema_version",
      "selection_table_version",
      "lifecycle_id",
      "program",
      "wave",
      "candidate_id",
      "content_id",
      "snapshot_sha256",
      "roster",
      "responses",
    ],
    "implementation response envelope",
  );
  if (envelope.artifact_kind !== RESPONSE_ARTIFACT) {
    error("implementation response envelope has an unexpected artifact_kind");
  }
  if (envelope.schema_version !== SELECTION_SCHEMA_VERSION ||
      envelope.selection_schema_version !== ledger.selection_schema_version ||
      envelope.selection_table_version !== ledger.selection_table_version) {
    error("implementation response envelope schema version disagrees with ledger");
  }
  for (const key of [
    "lifecycle_id",
    "program",
    "wave",
    "candidate_id",
    "content_id",
    "snapshot_sha256",
  ]) {
    if (envelope[key] !== ledger[key]) {
      error(`implementation response envelope ${key} disagrees with ledger`);
    }
  }
  if (
    !Array.isArray(envelope.roster) ||
    envelope.roster.join(",") !== ledger.roster.join(",")
  ) {
    error("implementation response envelope roster disagrees with ledger");
  }
  if (!Array.isArray(envelope.responses)) {
    error("implementation response envelope responses must be an array");
  }
  return envelope;
}

export function validateResponses(ledger, responses) {
  validateLedger(ledger);
  if (isPlainObject(responses) && Object.hasOwn(responses, "artifact_kind")) {
    validateResponseEnvelope(ledger, responses);
  }
  const byId = new Map(ledger.issues.map((issue) => [issue.id, issue]));
  const seen = new Set();
  const normalized = [];
  for (const [issueId, response] of responseEntries(responses)) {
    if (!byId.has(issueId)) error(`response references unknown issue "${issueId}"`);
    if (seen.has(issueId)) error(`duplicate implementation response for ${issueId}`);
    seen.add(issueId);
    if (!isPlainObject(response)) error(`response ${issueId} must be an object`);
    if (response.issue_id !== issueId) error(`response ${issueId} issue_id disagrees with its key`);
    if (!DISPOSITIONS.includes(response.disposition)) {
      error(
        `response ${issueId} disposition must be one of ${DISPOSITIONS.join(", ")}`,
      );
    }
    const justification = response.justification;
    const evidence = response.evidence;
    const disposition = response.disposition;
    const issue = byId.get(issueId);
    const surface = changedSurface(response, `response ${issueId}`);
    if (typeof justification !== "string" || justification.trim() === "") {
      error(`response ${issueId} requires a non-blank justification`);
    }
    if (
      (disposition === "Fixed" || disposition === "Invalid" || disposition === "Withdrawn") &&
      (typeof evidence !== "string" || evidence.trim() === "")
    ) {
      error(`response ${issueId} ${disposition} requires non-blank evidence`);
    }
    if (disposition === "Fixed" && surface.length === 0) {
      error(`response ${issueId} Fixed requires a changed_surface`);
    }
    const factualStatus =
      response.verified_factual_status ?? response.factual_status;
    if (
      (disposition === "Invalid" || disposition === "Withdrawn") &&
      (typeof factualStatus !== "string" || factualStatus.trim() === "")
    ) {
      error(`response ${issueId} ${disposition} requires verified_factual_status`);
    }
    if (
      issue.severity === "MAJOR" &&
      (disposition === "Intentionally rejected" || disposition === "Deferred") &&
      response.acceptance === undefined
    ) {
      error(
        `response ${issueId} unresolved MAJOR ${disposition} requires acceptance`,
      );
    }
    let acceptance;
    if (response.acceptance !== undefined) {
      if (
        !(
          (issue.severity === "MAJOR") &&
          (disposition === "Intentionally rejected" || disposition === "Deferred")
        )
      ) {
        error(`response ${issueId} cannot carry acceptance for this disposition`);
      }
      acceptance = validateAcceptance(response.acceptance, `response ${issueId}.acceptance`);
    }
    normalized.push({
      issue_id: issueId,
      disposition,
      justification,
      changed_surface: surface,
      ...(evidence !== undefined ? { evidence } : {}),
      ...(factualStatus !== undefined
        ? { verified_factual_status: factualStatus }
        : {}),
      ...(acceptance ? { acceptance } : {}),
    });
  }
  const missing = ledger.issues.map((issue) => issue.id).filter((id) => !seen.has(id));
  if (missing.length) error(`missing implementation responses for ${missing.join(", ")}`);
  return normalized.sort(
    (left, right) =>
      Number(left.issue_id.slice(1)) - Number(right.issue_id.slice(1)),
  );
}

export function validateSelfVerification(selfVerification) {
  const value = selfVerification?.self_verification ?? selfVerification;
  if (!isPlainObject(value)) error("self-verification must be an object");
  const required = [
    "tests",
    "lint",
    "formatting",
    "static_analysis",
    "build",
    "uncovered_areas",
    "self_review",
  ];
  for (const key of required) {
    if (!(key in value)) error(`self-verification is missing ${key}`);
    const item = value[key];
    if (
      item === null ||
      item === undefined ||
      (typeof item === "string" && item.trim() === "") ||
      (Array.isArray(item) && item.length === 0)
    ) {
      error(`self-verification ${key} must record an explicit result`);
    }
  }
  return sortedObject(value);
}

function assertCanonicalEqual(actual, expected, label) {
  if (stableStringify(actual) !== stableStringify(expected)) {
    error(`${label} disagrees with the exact staged artifact`);
  }
  return actual;
}

const SELECTION_SUMMARY_KEYS = [
  "candidate_id",
  "content_id",
  "lifecycle_id",
  "phase",
  "profiles",
  "program",
  "roster",
  "selection_schema_version",
  "selection_table_version",
  "snapshot_sha256",
  "wave",
];

function validateSelectionSummary(summary, table, label = "selection summary") {
  assertExactKeys(summary, SELECTION_SUMMARY_KEYS, label);
  if (!["discovery", "verification"].includes(summary.phase)) {
    error(`${label}.phase must be discovery or verification`);
  }
  safePathPart(summary.lifecycle_id, `${label}.lifecycle_id`);
  nonBlank(summary.program, `${label}.program`);
  nonBlank(summary.wave, `${label}.wave`);
  safePathPart(summary.candidate_id, `${label}.candidate_id`);
  nonBlank(summary.content_id, `${label}.content_id`);
  assertDigest(summary.snapshot_sha256, `${label}.snapshot_sha256`);
  if (summary.selection_schema_version !== SELECTION_SCHEMA_VERSION) {
    error(`${label}.selection_schema_version is unsupported`);
  }
  if (summary.selection_table_version !== SELECTION_TABLE_VERSION) {
    error(`${label}.selection_table_version is unsupported`);
  }
  const roster = validateRoster(summary.roster, table, `${label}.roster`);
  const canonicalRoster = seatOrder(table).filter((seat) => roster.includes(seat));
  if (roster.join(",") !== canonicalRoster.join(",")) {
    error(`${label}.roster is not in deterministic table order`);
  }
  if (!isPlainObject(summary.profiles)) {
    error(`${label}.profiles must be an object`);
  }
  const profileKeys = Object.keys(summary.profiles).sort();
  const expectedProfileKeys = [...roster].sort();
  if (
    profileKeys.length !== expectedProfileKeys.length ||
    profileKeys.some((key, index) => key !== expectedProfileKeys[index])
  ) {
    error(`${label}.profiles must contain exactly the roster seats`);
  }
  for (const seat of roster) {
    const profiles = summary.profiles[seat];
    if (!Array.isArray(profiles)) {
      error(`${label}.profiles.${seat} must be an array`);
    }
    const knownProfiles = new Set(Object.keys(table.seats[seat].profiles));
    if (profiles.some((profile) => typeof profile !== "string" || !knownProfiles.has(profile))) {
      error(`${label}.profiles.${seat} contains an unknown profile`);
    }
    const sortedProfiles = [...new Set(profiles)].sort();
    if (profiles.join(",") !== sortedProfiles.join(",")) {
      error(`${label}.profiles.${seat} must be unique and sorted`);
    }
  }
  return summary;
}

function validatePriorSelectionSummary(summary, selection, table) {
  if (!isPlainObject(summary)) {
    error("verification prior_selection must be a selection summary object");
  }
  validateSelectionSummary(summary, table, "verification prior_selection");
  if (summary.lifecycle_id !== selection.lifecycle_id) {
    error("verification prior_selection lifecycle_id disagrees with selection");
  }
  if (summary.program !== selection.program || summary.wave !== selection.wave) {
    error("verification prior_selection candidate lineage disagrees with selection");
  }
  validateMonotonicRoster(summary.roster, selection.roster, table);
  return summary;
}

function canonicalSelectionSummary(value, table, label) {
  if (!isPlainObject(value)) {
    error(`${label} must be a selection artifact or selection summary`);
  }
  if (value.artifact_kind === LIFECYCLE_SELECTION_ARTIFACT) {
    return selectionSummary(validateSelection(value, table), table);
  }
  return validateSelectionSummary(value, table, label);
}

export function validateDiscoveryRequest(request, options = {}) {
  const table = options.table ?? readSelectionTable(options.table_path);
  const selection = options.selection;
  const currentCandidate =
    options.current_candidate ??
    options.currentCandidate ??
    options.candidate;
  if (!selection) error("discovery request validation requires the exact selection");
  if (!currentCandidate) {
    error("discovery request validation requires the exact current candidate");
  }
  validateSelection(selection, table);
  if (selection.phase !== "discovery") {
    error("discovery request validation requires a discovery selection");
  }
  validateCandidateAgainstSelection(selection, currentCandidate, table);
  assertExactKeys(
    request,
    [
      "artifact_kind",
      "schema_version",
      "lifecycle_id",
      "phase",
      "selection",
      "candidate",
      "full_candidate",
      "comprehensive",
      "instruction",
      "context",
      "validation_evidence",
      "seats",
    ],
    "discovery request",
  );
  if (request.artifact_kind !== DISCOVERY_REQUEST_ARTIFACT) {
    error("discovery request has an unexpected artifact_kind");
  }
  if (request.schema_version !== SELECTION_SCHEMA_VERSION) {
    error("discovery request schema_version is unsupported");
  }
  if (request.lifecycle_id !== selection.lifecycle_id) {
    error("discovery request lifecycle_id disagrees with selection");
  }
  if (request.phase !== "discovery") {
    error("discovery request phase must be discovery");
  }
  validateSelectionSummary(request.selection, table, "discovery request selection");
  assertCanonicalEqual(
    request.selection,
    selectionSummary(selection, table),
    "discovery request selection",
  );
  assertCanonicalEqual(
    request.candidate,
    candidateAddress(currentCandidate),
    "discovery request candidate",
  );
  if (request.full_candidate !== true || request.comprehensive !== true) {
    error("discovery request must be a comprehensive full-candidate request");
  }
  if (
    typeof request.instruction !== "string" ||
    request.instruction.trim() === ""
  ) {
    error("discovery request instruction must be non-blank");
  }
  if (!isPlainObject(request.context)) {
    error("discovery request context must be an object");
  }
  if (!Array.isArray(request.validation_evidence)) {
    error("discovery request validation_evidence must be an array");
  }
  const expected = createDiscoveryRequest({
    selection,
    candidate: currentCandidate,
    context: request.context,
    validation_evidence: request.validation_evidence,
  }, { table });
  assertCanonicalEqual(request, expected, "discovery request");
  return request;
}

const VERIFICATION_REQUEST_KEYS = [
  "artifact_kind",
  "schema_version",
  "lifecycle_id",
  "phase",
  "seat",
  "selection",
  "comprehensive_discovery_already_complete",
  "instruction",
  "discovery_ledger",
  "ledger",
  "responses",
  "self_verification",
  "latest_delta_paths",
  "actual_delta",
  "current_candidate",
  "full_candidate",
  "current_selection",
  "fix_delta",
  "prior_selection",
  "previous_status",
  "obligations",
];

const SHARED_VERIFICATION_REQUEST_KEYS = [
  "selection",
  "discovery_ledger",
  "ledger",
  "responses",
  "self_verification",
  "latest_delta_paths",
  "actual_delta",
  "current_candidate",
  "full_candidate",
  "current_selection",
  "fix_delta",
  "prior_selection",
];

function validateVerificationFixDelta(fixDelta, expectedPaths, selection) {
  if (!isPlainObject(fixDelta)) {
    error("verification request fix_delta must be an object");
  }
  const allowedKeys = new Set([
    "changed_paths",
    "signals",
    "candidate_class",
    "ambiguous",
  ]);
  const unknown = Object.keys(fixDelta).filter((key) => !allowedKeys.has(key));
  if (unknown.length > 0) {
    error(`verification request fix_delta contains unknown field(s): ${unknown.join(", ")}`);
  }
  const actual = candidateInputs(fixDelta);
  if (actual.changed_paths.join("\u0000") !== expectedPaths.join("\u0000")) {
    error("verification request fix_delta changed_paths disagree with selection");
  }
  const expected = selection.classification_inputs.fix_delta;
  if (
    Object.hasOwn(fixDelta, "signals") &&
    actual.signals.join("\u0000") !== expected.signals.join("\u0000")
  ) {
    error("verification request fix_delta signals disagree with selection");
  }
  if (
    Object.hasOwn(fixDelta, "candidate_class") &&
    actual.candidate_class !== expected.candidate_class
  ) {
    error("verification request fix_delta candidate_class disagrees with selection");
  }
  if (
    Object.hasOwn(fixDelta, "ambiguous") &&
    actual.ambiguous !== expected.ambiguous
  ) {
    error("verification request fix_delta ambiguity disagrees with selection");
  }
  return fixDelta;
}

export function validateVerificationRequest(request, options = {}) {
  const table = options.table ?? readSelectionTable(options.table_path);
  const selection = options.selection;
  const ledger = options.ledger ?? options.discovery_ledger;
  const responses = options.responses;
  const selfVerification =
    options.self_verification ?? options.selfVerification;
  const currentCandidate =
    options.current_candidate ??
    options.currentCandidate ??
    options.candidate;
  const hasCanonicalActualDelta =
    Object.hasOwn(options, "actual_delta") ||
    Object.hasOwn(options, "actualDelta");
  const canonicalActualDelta =
    options.actual_delta ?? options.actualDelta;
  const hasCanonicalPriorSelection =
    Object.hasOwn(options, "prior_selection") ||
    Object.hasOwn(options, "priorSelection");
  const canonicalPriorSelection =
    options.prior_selection ?? options.priorSelection;
  const hasCanonicalPreviousStatus = Object.hasOwn(options, "previous_status");
  const canonicalPreviousStatus = options.previous_status;
  if (!selection) error("verification request validation requires the exact selection");
  if (!ledger) error("verification request validation requires the exact immutable ledger");
  if (responses === undefined) {
    error("verification request validation requires the exact responses");
  }
  if (selfVerification === undefined) {
    error("verification request validation requires the exact self-verification");
  }
  if (!currentCandidate) {
    error("verification request validation requires the exact current candidate");
  }
  validateSelection(selection, table);
  if (selection.phase !== "verification") {
    error("verification request validation requires a verification selection");
  }
  validateCandidateAgainstSelection(selection, currentCandidate, table);
  validateLedger(ledger, { table, selection });
  const normalizedResponses = validateResponses(ledger, responses);
  const normalizedSelfVerification = validateSelfVerification(selfVerification);
  assertExactKeys(request, VERIFICATION_REQUEST_KEYS, "verification request");
  if (request.artifact_kind !== VERIFICATION_ARTIFACT) {
    error("verification request has an unexpected artifact_kind");
  }
  if (request.schema_version !== SELECTION_SCHEMA_VERSION) {
    error("verification request schema_version is unsupported");
  }
  if (request.lifecycle_id !== selection.lifecycle_id) {
    error("verification request lifecycle_id disagrees with selection");
  }
  if (request.phase !== "verification") {
    error("verification request phase must be verification");
  }
  if (!selection.roster.includes(request.seat)) {
    error(`verification request is for unselected seat "${request.seat}"`);
  }
  const summary = selectionSummary(selection, table);
  validateSelectionSummary(request.selection, table, "verification request selection");
  validateSelectionSummary(
    request.current_selection,
    table,
    "verification request current_selection",
  );
  assertCanonicalEqual(
    request.selection,
    summary,
    "verification request selection",
  );
  assertCanonicalEqual(
    request.current_selection,
    summary,
    "verification request current_selection",
  );
  if (request.comprehensive_discovery_already_complete !== true) {
    error("verification request must state that comprehensive discovery is complete");
  }
  const expectedInstruction =
    "Verify prior findings, responses, evidence, and regressions, including a new surface that selected this seat. " +
    "Do not reopen the whole review unless an introduced regression or a previously missed BLOCKER or MAJOR makes approval unsafe.";
  if (request.instruction !== expectedInstruction) {
    error("verification request instruction is not the canonical scoped-verification instruction");
  }
  assertCanonicalEqual(
    request.discovery_ledger,
    ledger,
    "verification request discovery_ledger",
  );
  assertCanonicalEqual(request.ledger, ledger, "verification request ledger");
  assertCanonicalEqual(
    request.responses,
    normalizedResponses,
    "verification request responses",
  );
  assertCanonicalEqual(
    request.self_verification,
    normalizedSelfVerification,
    "verification request self-verification",
  );
  const declaredDeltaPaths =
    selection.classification_inputs.fix_delta.changed_paths;
  if (
    !Array.isArray(request.latest_delta_paths) ||
    request.latest_delta_paths.length === 0 ||
    (declaredDeltaPaths.length > 0 &&
      request.latest_delta_paths.join("\u0000") !== declaredDeltaPaths.join("\u0000"))
  ) {
    error("verification request latest_delta_paths disagree with selection");
  }
  const expectedDeltaPaths = request.latest_delta_paths;
  assertExactKeys(
    request.actual_delta,
    ["paths"],
    "verification request actual_delta",
  );
  if (
    !Array.isArray(request.actual_delta.paths) ||
    request.actual_delta.paths.join("\u0000") !== expectedDeltaPaths.join("\u0000")
  ) {
    error("verification request actual_delta paths disagree with selection");
  }
  assertCanonicalEqual(
    request.current_candidate,
    candidateAddress(currentCandidate),
    "verification request current_candidate",
  );
  assertCanonicalEqual(
    request.full_candidate,
    candidateAddress(currentCandidate),
    "verification request full_candidate",
  );
  validateVerificationFixDelta(
    request.fix_delta,
    expectedDeltaPaths,
    selection,
  );
  validatePriorSelectionSummary(request.prior_selection, selection, table);
  if (hasCanonicalActualDelta) {
    assertCanonicalEqual(
      request.actual_delta,
      canonicalActualDelta,
      "verification request actual_delta",
    );
  }
  if (hasCanonicalPriorSelection) {
    const expectedPriorSelection = canonicalSelectionSummary(
      canonicalPriorSelection,
      table,
      "canonical verification prior_selection",
    );
    assertCanonicalEqual(
      request.prior_selection,
      expectedPriorSelection,
      "verification request prior_selection",
    );
  }
  const incumbent = request.prior_selection.roster.includes(request.seat);
  if (incumbent && !isPlainObject(request.previous_status)) {
    error(
      `verification request incumbent seat "${request.seat}" must carry its prior verdict`,
    );
  }
  if (!incumbent && request.previous_status !== null) {
    error(
      `verification request newly selected seat "${request.seat}" must carry a null prior status`,
    );
  }
  if (
    request.previous_status !== null &&
    request.previous_status !== undefined &&
    !isPlainObject(request.previous_status)
  ) {
    error("verification request previous_status must be null or an object");
  }
  if (hasCanonicalPreviousStatus) {
    assertCanonicalEqual(
      request.previous_status,
      canonicalPreviousStatus,
      "verification request previous_status",
    );
  }
  assertExactKeys(
    request.obligations,
    ["focus", "profiles"],
    "verification request obligations",
  );
  if (request.obligations.focus !== table.seats[request.seat].focus) {
    error("verification request focus disagrees with the selected seat profile");
  }
  assertCanonicalEqual(
    request.obligations.profiles,
    selection.profiles[request.seat],
    "verification request profiles",
  );
  return request;
}

export function validateVerificationRequests(
  selection,
  requests,
  options = {},
) {
  const table = options.table ?? readSelectionTable(options.table_path);
  const entries = Array.isArray(requests)
    ? requests.map((request) => [request?.seat, request])
    : isPlainObject(requests)
      ? Object.entries(requests).map(([seat, request]) => [seat, request])
      : error("verification requests must be an array or object keyed by seat");
  const expected = new Set(selection.roster);
  const seen = new Set();
  let canonicalSharedRequest;
  const validated = [];
  const priorSelectionOption =
    options.prior_selection ?? options.priorSelection;
  const previousStatuses =
    options.previous_statuses ?? options.previousStatuses;
  let priorSummary;
  if (priorSelectionOption !== undefined) {
    priorSummary = canonicalSelectionSummary(
      priorSelectionOption,
      table,
      "canonical verification prior_selection",
    );
    if (!isPlainObject(previousStatuses)) {
      error(
        "verification requests require prior statuses for every incumbent seat",
      );
    }
    const actualPriorStatusKeys = Object.keys(previousStatuses).sort();
    const expectedPriorStatusKeys = [...priorSummary.roster].sort();
    if (
      actualPriorStatusKeys.length !== expectedPriorStatusKeys.length ||
      actualPriorStatusKeys.some(
        (key, index) => key !== expectedPriorStatusKeys[index],
      )
    ) {
      error(
        "verification requests prior statuses must contain exactly the incumbent seats",
      );
    }
  }
  for (const [seat, request] of entries) {
    if (seat !== request?.seat) {
      error(`verification request key "${seat}" disagrees with its declared seat`);
    }
    if (!expected.has(seat)) {
      error(`verification request for unselected seat "${seat}"`);
    }
    if (seen.has(seat)) {
      error(`duplicate verification request for seat "${seat}"`);
    }
    seen.add(seat);
    const requestOptions = {
      ...options,
      table,
      selection,
      current_candidate:
        options.current_candidate ??
        options.currentCandidate ??
        options.candidate,
      ledger: options.ledger ?? options.discovery_ledger,
      responses: options.responses,
      self_verification:
        options.self_verification ?? options.selfVerification,
    };
    if (priorSelectionOption !== undefined) {
      requestOptions.prior_selection = priorSelectionOption;
      requestOptions.previous_status = priorSummary.roster.includes(seat)
        ? previousStatuses[seat]
        : null;
    } else if (isPlainObject(previousStatuses) &&
               Object.hasOwn(previousStatuses, seat)) {
      requestOptions.previous_status = previousStatuses[seat];
    }
    if (
      Object.hasOwn(options, "actual_delta") ||
      Object.hasOwn(options, "actualDelta")
    ) {
      requestOptions.actual_delta = options.actual_delta ?? options.actualDelta;
    }
    if (
      Object.hasOwn(options, "prior_selection") ||
      Object.hasOwn(options, "priorSelection")
    ) {
      requestOptions.prior_selection =
        options.prior_selection ?? options.priorSelection;
    }
    validateVerificationRequest(request, requestOptions);
    if (canonicalSharedRequest === undefined) {
      canonicalSharedRequest = request;
    } else {
      for (const key of SHARED_VERIFICATION_REQUEST_KEYS) {
        assertCanonicalEqual(
          request[key],
          canonicalSharedRequest[key],
          `verification request ${seat} shared ${key}`,
        );
      }
    }
    validated.push(request);
  }
  const missing = selection.roster.filter((seat) => !seen.has(seat));
  if (missing.length > 0) {
    error(
      `verification requests are missing selected seat(s): ${missing.join(", ")}`,
    );
  }
  return validated;
}

export function validateStagedRoundArtifacts(input, options = {}) {
  if (!isPlainObject(input)) {
    error("staged panel artifacts must be an object");
  }
  const table = options.table ?? readSelectionTable(options.table_path);
  const selection = input.selection;
  const currentCandidate =
    input.current_candidate ??
    input.currentCandidate ??
    input.candidate;
  if (!selection) error("staged panel artifacts require selection");
  if (!currentCandidate) error("staged panel artifacts require current candidate");
  validateSelection(selection, table);
  validateCandidateAgainstSelection(selection, currentCandidate, table);
  if (input.phase !== undefined && input.phase !== selection.phase) {
    error("staged panel artifact phase disagrees with selection");
  }
  if (input.lifecycle_id !== undefined && input.lifecycle_id !== selection.lifecycle_id) {
    error("staged panel artifact lifecycle_id disagrees with selection");
  }
  if (selection.phase === "discovery") {
    if (!input.discovery_request) {
      error("staged discovery artifacts require discovery_request");
    }
    validateDiscoveryRequest(input.discovery_request, {
      table,
      selection,
      current_candidate: currentCandidate,
    });
    return { phase: selection.phase, roster: [...selection.roster] };
  }
  const ledger = input.ledger ?? input.discovery_ledger;
  if (!ledger) error("staged verification artifacts require immutable ledger");
  if (input.responses === undefined) {
    error("staged verification artifacts require responses");
  }
  if (input.self_verification === undefined && input.selfVerification === undefined) {
    error("staged verification artifacts require self-verification");
  }
  validateLedger(ledger, { table, selection });
  validateResponses(ledger, input.responses);
  validateSelfVerification(input.self_verification ?? input.selfVerification);
  const verificationRequests =
    input.verification_requests ?? input.verificationRequests;
  if (!verificationRequests) {
    error("staged verification artifacts require verification requests");
  }
  const requestValues = Array.isArray(verificationRequests)
    ? verificationRequests
    : Object.values(verificationRequests);
  const canonicalRequest = requestValues[0];
  if (!isPlainObject(canonicalRequest)) {
    error("staged verification artifacts require JSON verification requests");
  }
  validateVerificationRequests(selection, verificationRequests, {
    table,
    current_candidate: currentCandidate,
    ledger,
    responses: input.responses,
    self_verification: input.self_verification ?? input.selfVerification,
    actual_delta:
      options.actual_delta ??
      options.actualDelta ??
      canonicalRequest.actual_delta,
    prior_selection:
      options.prior_selection ??
      options.priorSelection ??
      canonicalRequest.prior_selection,
    ...((options.previous_statuses ?? options.previousStatuses)
      ? { previous_statuses: options.previous_statuses ?? options.previousStatuses }
      : {}),
  });
  return { phase: selection.phase, roster: [...selection.roster] };
}

function pathCovered(path, declared) {
  return declared.some((entry) => {
    if (entry.endsWith("/")) return path === entry.slice(0, -1) || path.startsWith(entry);
    return path === entry || path.startsWith(`${entry}/`);
  });
}

export function validateFixScope(input) {
  const latestDelta = input.latest_delta_paths ??
    input.fix_delta_paths ??
    input.changed_paths ??
    [];
  if (!Array.isArray(latestDelta) || latestDelta.some((path) => typeof path !== "string")) {
    error("latest_delta_paths must be an array of strings");
  }
  for (const path of latestDelta) {
    if (path.trim() === "" || CONTROL_CHARACTER_PATTERN.test(path)) {
      error("latest_delta_paths must contain non-blank paths without control characters");
    }
    utf8Bytes(path, "latest delta path");
  }
  const explicit = input.allowed_paths ?? input.scope?.allowed_paths;
  let allowed;
  if (explicit !== undefined) {
    if (!Array.isArray(explicit) || explicit.some((path) => typeof path !== "string")) {
      error("allowed_paths must be an array of strings");
    }
    for (const path of explicit) {
      if (path.trim() === "" || CONTROL_CHARACTER_PATTERN.test(path)) {
        error("allowed_paths must contain non-blank paths without control characters");
      }
      utf8Bytes(path, "allowed path");
    }
    allowed = explicit;
  } else {
    const responses = input.responses ?? [];
    if (!Array.isArray(responses)) error("responses must be an array for scope validation");
    allowed = responses.flatMap((response) => response.changed_surface ?? []);
  }
  const outside = latestDelta.filter((path) => !pathCovered(path, allowed));
  if (outside.length) {
    error(
      `fix scope contains unrelated paths: ${outside.join(", ")}; start or explicitly rescope a new lifecycle`,
    );
  }
  return {
    latest_delta_paths: sortUtf8([...new Set(latestDelta)]),
    allowed_paths: sortUtf8([...new Set(allowed)]),
  };
}

export function lateFindingAdmission(finding) {
  if (!isPlainObject(finding)) error("late finding must be an object");
  const severity = verdictSeverity(finding.severity, "late finding severity");
  const introduced = finding.introduced_regression === true || finding.introduced === true;
  const missed = finding.previously_missed === true || finding.missed_discovery === true;
  const unsafeClass = ["correctness", "security", "data-loss", "reliability"].includes(
    String(finding.category ?? "").toLowerCase(),
  );
  const admitted = introduced || (missed && ["BLOCKER", "MAJOR"].includes(severity)) ||
    (unsafeClass && ["BLOCKER", "MAJOR"].includes(severity));
  if (!admitted) {
    error(
      `late ${severity} finding is not admissible during scoped verification; pre-existing MINOR/NIT and optional improvements do not reopen discovery`,
    );
  }
  return {
    ...finding,
    severity,
    late: true,
    admission_reason: introduced
      ? "introduced-regression"
      : missed
        ? "previously-missed-merge-risk"
        : "unsafe-merge-risk",
  };
}

export function appendLateFindings(ledger, findings) {
  validateLedger(ledger);
  if (!Array.isArray(findings)) error("late findings must be an array");
  const issues = ledger.issues.map((issue) => ({ ...issue }));
  const sources = ledger.sources.map((source) => ({ ...source }));
  let nextId = issues.length + 1;
  for (const finding of findings) {
    const admitted = lateFindingAdmission(finding);
    const rawLateText = admitted.raw_text ?? admitted.text ?? admitted.description;
    const sourceId =
      admitted.source_id ??
      `late:${sha256({
        seat: admitted.seat ?? "verification",
        raw_text: rawLateText,
      })}`;
    if (sources.some((source) => source.source_id === sourceId)) {
      error(`late source finding ${sourceId} already exists`);
    }
    const rawText = rawLateText;
    nonBlank(rawText, `late finding ${sourceId}.raw_text`);
    const source = {
      source_id: sourceId,
      seat: nonBlank(admitted.seat ?? "verification", `late finding ${sourceId}.seat`),
      source_ordinal: admitted.source_ordinal ?? nextId,
      raw_text: rawText,
      attribution: admitted.attribution ?? admitted.seat ?? "verification",
      severity: admitted.severity,
      impact: nonBlank(admitted.impact ?? "Late finding makes approval unsafe.", `late finding ${sourceId}.impact`),
      recommendation: nonBlank(admitted.recommendation ?? admitted.fix ?? rawText, `late finding ${sourceId}.recommendation`),
    };
    sources.push(source);
    issues.push({
      id: `R${nextId}`,
      description: nonBlank(admitted.description ?? rawText, `R${nextId}.description`),
      severity: admitted.severity,
      impact: source.impact,
      recommendation: source.recommendation,
      source_finding_ids: [sourceId],
      late: true,
    });
    nextId += 1;
  }
  return {
    ...ledger,
    sources,
    issues,
  };
}

function verificationEntries(results) {
  const value = results?.results ?? results?.verdicts ?? results;
  if (Array.isArray(value)) return value.map((result) => [result?.seat, result]);
  if (isPlainObject(value)) {
    return Object.entries(value).map(([seat, result]) => [
      result?.seat ?? seat,
      { ...(isPlainObject(result) ? result : {}), seat: result?.seat ?? seat },
    ]);
  }
  error("verification results must be an array or object keyed by seat");
}

function exactIssueStatuses(ledger, statuses, label) {
  if (!isPlainObject(statuses)) {
    error(`${label} must be an object keyed by every ledger issue`);
  }
  const expected = ledger.issues.map((issue) => issue.id);
  const actual = Object.keys(statuses).sort();
  const wanted = [...expected].sort();
  if (actual.length !== wanted.length || actual.some((key, index) => key !== wanted[index])) {
    const missing = expected.filter((key) => !Object.hasOwn(statuses, key));
    const extra = actual.filter((key) => !expected.includes(key));
    error(
      `${label} must cover each issue exactly once; missing [${missing.join(", ")}], ` +
      `extra [${extra.join(", ")}]`,
    );
  }
  for (const issueId of expected) {
    const status = statuses[issueId];
    const statusText = typeof status === "string"
      ? status.trim().toLowerCase()
      : isPlainObject(status) && typeof status.status === "string"
        ? status.status.trim().toLowerCase()
        : "";
    if (!VERIFICATION_STATUSES.includes(statusText)) {
      error(
        `${label}.${issueId}.status must be one of ${VERIFICATION_STATUSES.join(", ")}`,
      );
    }
    if (typeof status === "string") {
      if (status.trim() === "") error(`${label}.${issueId} must be non-blank`);
      continue;
    }
    if (!isPlainObject(status)) {
      error(`${label}.${issueId} must be a non-blank status string or object`);
    }
    const statusKeys = Object.keys(status).sort();
    if (
      statusKeys.length === 0 ||
      !statusKeys.includes("status") ||
      statusKeys.some((key) => !["status", "evidence"].includes(key))
    ) {
      error(`${label}.${issueId} must contain only status and optional evidence`);
    }
    if (typeof status.status !== "string" || status.status.trim() === "") {
      error(`${label}.${issueId}.status must be non-blank`);
    }
    if (
      status.evidence !== undefined &&
      (typeof status.evidence !== "string" || status.evidence.trim() === "")
    ) {
      error(`${label}.${issueId}.evidence must be non-blank when present`);
    }
  }
  return sortedObject(statuses);
}

export function validateVerificationResults(selection, results, options = {}) {
  const table = options.table ?? readSelectionTable(options.table_path);
  validateSelection(selection, table);
  const ledger = options.ledger;
  if (!ledger) error("verification validation requires the immutable discovery ledger");
  validateLedger(ledger);
  validateMonotonicRoster(ledger.roster, selection.roster, table);
  const rawEntries = verificationEntries(results);
  const actualVerdicts = rawEntries.length > 0 &&
    rawEntries.every(([, result]) =>
      isPlainObject(result) &&
      Object.hasOwn(result, "engineer") &&
      !Object.hasOwn(result, "complete"),
    );
  const adapted = actualVerdicts
    ? Object.fromEntries(
        adaptVerificationResults(results, {
          issue_ids: ledger.issues.map((issue) => issue.id),
        }).map((result) => [result.seat, result]),
      )
    : results;
  if (
    rawEntries.some(([, result]) =>
      isPlainObject(result) &&
      Object.hasOwn(result, "engineer") &&
      !Object.hasOwn(result, "complete"),
    ) !== actualVerdicts
  ) {
    error("verification results must not mix actual verdict JSON with verification result shapes");
  }
  const expected = new Set(selection.roster);
  const seen = new Set();
  const normalized = [];
  for (const [seat, result] of verificationEntries(adapted)) {
    if (!expected.has(seat)) error(`verification result for unselected seat "${seat}"`);
    if (seen.has(seat)) error(`duplicate verification result for seat "${seat}"`);
    seen.add(seat);
    if (!isPlainObject(result) || result.complete !== true) {
      error(`verification result for ${seat} must explicitly set complete: true`);
    }
    if (typeof result.signoff !== "boolean") {
      error(`verification result for ${seat} must explicitly set signoff`);
    }
    if (!Array.isArray(result.recommendations)) {
      error(`verification result for ${seat} must explicitly contain recommendations`);
    }
    const recommendations = result.recommendations;
    if (!Array.isArray(recommendations)) {
      error(`verification result for ${seat} recommendations must be an array`);
    }
    if (result.signoff !== (recommendations.length === 0)) {
      error(`verification result for ${seat} signoff must equal recommendations.isEmpty`);
    }
    const statuses = result.verified_issue_statuses ??
      result.issue_statuses ??
      result.verification_statuses;
    const verifiedIssueStatuses = exactIssueStatuses(
      ledger,
      statuses,
      `verification ${seat}.verified_issue_statuses`,
    );
    const late = (result.late_findings ?? []).map(lateFindingAdmission);
    normalized.push({
      seat,
      complete: true,
      signoff: recommendations.length === 0,
      recommendations,
      verified_issue_statuses: verifiedIssueStatuses,
      blocking_recommendations: recommendations,
      late_findings: late,
      summary: nonBlank(result.summary ?? "Verification complete.", `verification ${seat}.summary`),
    });
  }
  for (const seat of selection.roster) {
    if (!seen.has(seat)) error(`missing verification result for selected seat "${seat}"`);
  }
  return normalized.sort(
    (left, right) => selection.roster.indexOf(left.seat) - selection.roster.indexOf(right.seat),
  );
}

function normalizePriorVerdicts(input, priorSelection) {
  const verdicts = typeof input === "string"
    ? readJsonDirectory(input, "prior verdicts")
    : input;
  if (!isPlainObject(verdicts)) {
    error("verification preparation prior verdicts must be an object keyed by seat");
  }
  const expectedSeats = [...priorSelection.roster].sort();
  const actualSeats = Object.keys(verdicts).sort();
  if (
    actualSeats.length !== expectedSeats.length ||
    actualSeats.some((seat, index) => seat !== expectedSeats[index])
  ) {
    error(
      "verification preparation prior verdicts must contain exactly one verdict for every prior seat",
    );
  }
  for (const seat of priorSelection.roster) {
    const verdict = verdicts[seat];
    if (!isPlainObject(verdict)) {
      error(`verification preparation prior verdict for ${seat} must be an object`);
    }
    const declaredSeat = verdict.engineer ?? verdict.seat;
    if (declaredSeat !== seat) {
      error(
        `verification preparation prior verdict ${seat} declares seat "${declaredSeat}"`,
      );
    }
  }
  return verdicts;
}

export function prepareVerification(input, options = {}) {
  const table = options.table ?? readSelectionTable(options.table_path);
  const selection =
    input.current_selection ??
    input.currentSelection ??
    input.selection ??
    (input.selection_path
      ? readSelection(input.selection_path, { table })
      : undefined);
  if (!selection) error("verification preparation requires a current selection");
  validateSelection(selection, table);
  if (selection.phase !== "verification") {
    error("verification preparation requires a verification selection");
  }
  if (
    input.current_candidate === undefined &&
    input.currentCandidate === undefined &&
    input.candidate === undefined
  ) {
    error("verification preparation requires an explicit current candidate");
  }
  const discoveryLedger = input.discovery_ledger ?? input.discoveryLedger ?? input.ledger;
  validateLedger(discoveryLedger);
  if (discoveryLedger.lifecycle_id !== selection.lifecycle_id) {
    error("discovery ledger and verification selection lifecycle_id disagree");
  }
  const currentCandidate =
    input.current_candidate ??
    input.currentCandidate ??
    input.candidate ??
    candidateFromSelection(selection, { table });
  validateSelectionCandidate(selection, currentCandidate);
  validateMonotonicRoster(discoveryLedger.roster, selection.roster, table);
  const priorSelectionInput =
    input.prior_selection ??
    input.previous_selection ??
    input.priorSelection ??
    input.previousSelection;
  if (priorSelectionInput === undefined || priorSelectionInput === null) {
    error("verification preparation requires an explicit prior selection");
  }
  const priorSelection = typeof priorSelectionInput === "string"
    ? readSelection(priorSelectionInput, { table })
    : priorSelectionInput;
  if (priorSelection) {
    validateSelection(priorSelection, table);
    if (priorSelection.lifecycle_id !== selection.lifecycle_id) {
      error("prior selection and verification selection lifecycle_id disagree");
    }
    validateMonotonicRoster(priorSelection.roster, selection.roster, table);
  }
  const priorVerdictsInput =
    input.prior_verdicts ??
    input.priorVerdicts;
  if (priorVerdictsInput === undefined || priorVerdictsInput === null) {
    error("verification preparation requires explicit prior verdicts");
  }
  const priorVerdicts = normalizePriorVerdicts(
    priorVerdictsInput,
    priorSelection,
  );
  const responses = validateResponses(discoveryLedger, input.responses);
  const selfVerification = validateSelfVerification(
    input.self_verification ?? input.selfVerification,
  );
  const actualDeltaPaths =
    input.actual_delta_paths ??
    input.latest_delta_paths ??
    input.fix_delta_paths ??
    undefined;
  if (!Array.isArray(actualDeltaPaths) || actualDeltaPaths.length === 0) {
    error("verification preparation requires a non-empty actual delta");
  }
  if (actualDeltaPaths.some(
    (path) =>
      typeof path !== "string" ||
      path.trim() === "" ||
      CONTROL_CHARACTER_PATTERN.test(path),
  )) {
    error("verification preparation actual delta paths must be non-blank strings");
  }
  const scope = validateFixScope({
    ...input,
    latest_delta_paths: actualDeltaPaths,
    responses,
  });
  const requests = selection.roster.map((seat) => ({
    artifact_kind: VERIFICATION_ARTIFACT,
    schema_version: SELECTION_SCHEMA_VERSION,
    lifecycle_id: selection.lifecycle_id,
    phase: "verification",
    seat,
    selection: selectionSummary(selection, table),
    comprehensive_discovery_already_complete: true,
    instruction:
      "Verify prior findings, responses, evidence, and regressions, including a new surface that selected this seat. Do not reopen the whole review unless an introduced regression or a previously missed BLOCKER or MAJOR makes approval unsafe.",
    discovery_ledger: discoveryLedger,
    ledger: discoveryLedger,
    responses,
    self_verification: selfVerification,
    latest_delta_paths: scope.latest_delta_paths,
    actual_delta: {
      paths: scope.latest_delta_paths,
    },
    current_candidate: candidateAddress(currentCandidate),
    full_candidate: candidateAddress(currentCandidate),
    current_selection: selectionSummary(selection, table),
    fix_delta: input.fix_delta ?? input.fixDelta ?? {
      changed_paths: scope.latest_delta_paths,
    },
    prior_selection: priorSelection
      ? selectionSummary(priorSelection, table)
      : null,
    previous_status: priorSelection.roster.includes(seat)
      ? priorVerdicts[seat]
      : null,
    obligations: {
      focus: table.seats[seat].focus,
      profiles: selection.profiles[seat],
    },
  }));
  return {
    artifact_kind: VERIFICATION_ARTIFACT,
    schema_version: SELECTION_SCHEMA_VERSION,
    lifecycle_id: selection.lifecycle_id,
    phase: "verification",
    selection: selectionSummary(selection, table),
    discovery_ledger: discoveryLedger,
    ledger: discoveryLedger,
    responses,
    self_verification: selfVerification,
    scope,
    current_candidate: candidateAddress(currentCandidate),
    full_candidate: candidateAddress(currentCandidate),
    current_selection: selectionSummary(selection, table),
    fix_delta: input.fix_delta ?? input.fixDelta ?? {
      changed_paths: scope.latest_delta_paths,
    },
    prior_selection: priorSelection
      ? selectionSummary(priorSelection, table)
      : null,
    requests,
  };
}

export function writeVerificationArtifacts(outputDir, input, options = {}) {
  const prepared = prepareVerification(input, options);
  const publication = writeDirectoryCreateOrCompare(
    outputDir,
    prepared.requests.map((request) => ({
      name: `${request.seat}.json`,
      bytes: stableStringify(request),
    })),
  );
  return {
    prepared,
    publication,
    written: prepared.requests.map((request) => ({
      path: join(outputDir, `${request.seat}.json`),
      created: publication.created,
    })),
  };
}

function statusName(value) {
  if (typeof value === "string") return value.trim().toLowerCase();
  if (isPlainObject(value) && typeof value.status === "string") {
    return value.status.trim().toLowerCase();
  }
  return "";
}

const PASSING_VERIFICATION_STATUSES = new Set(["resolved", "verified"]);

function verificationStatusBlocks(verification) {
  return verification.flatMap((result) =>
    Object.entries(result.verified_issue_statuses)
      .filter(([, status]) => !PASSING_VERIFICATION_STATUSES.has(statusName(status)))
      .map(([issueId, status]) => ({
        seat: result.seat,
        issue_id: issueId,
        status,
      })),
  );
}

function lateFindingsFromVerification(verification) {
  return verification.flatMap((result) =>
    result.late_findings.map((finding) => ({
      ...finding,
      seat: finding.seat ?? result.seat,
      attribution: finding.attribution ?? result.seat,
    })),
  );
}

function responseApproves(issue, response) {
  if (issue.severity === "BLOCKER") {
    return response.disposition === "Fixed" ||
      ((response.disposition === "Invalid" || response.disposition === "Withdrawn") &&
        typeof response.verified_factual_status === "string" &&
        response.verified_factual_status.trim() !== "");
  }
  if (issue.severity === "MAJOR") {
    if (response.disposition === "Fixed") return true;
    if (
      response.disposition === "Invalid" ||
      response.disposition === "Withdrawn"
    ) {
      return typeof response.verified_factual_status === "string" &&
        response.verified_factual_status.trim() !== "";
    }
    return (
      (response.disposition === "Intentionally rejected" ||
        response.disposition === "Deferred") &&
      response.acceptance !== undefined
    );
  }
  return true;
}

export function evaluateApproval(input) {
  const table = input.table ?? readSelectionTable(input.table_path);
  const selection = input.current_selection ?? input.selection;
  validateSelection(selection, table);
  const currentCandidate =
    input.current_candidate ??
    input.currentCandidate ??
    input.candidate ??
    candidateFromSelection(selection, { table });
  validateSelectionCandidate(selection, currentCandidate);
  const ledger = input.discovery_ledger ?? input.ledger;
  validateLedger(ledger);
  validateMonotonicRoster(ledger.roster, selection.roster, table);
  const responses = validateResponses(ledger, input.responses);
  const responseById = new Map(responses.map((response) => [response.issue_id, response]));
  const blockingIssues = ledger.issues
    .filter((issue) => !responseApproves(issue, responseById.get(issue.id)))
    .map((issue) => issue.id);
  const verification = input.verification_results
    ? validateVerificationResults(selection, input.verification_results, { table, ledger })
    : [];
  const verificationBlocks = verification.flatMap((result) =>
    result.blocking_recommendations.map((recommendation) => ({
      seat: result.seat,
      recommendation,
    })),
  );
  const lateFindings = lateFindingsFromVerification(verification);
  const ledgerWithLate = lateFindings.length
    ? appendLateFindings(ledger, lateFindings)
    : ledger;
  const lateIssues = ledgerWithLate.issues.filter(
    (issue) => issue.late && !ledger.issues.some((existing) => existing.id === issue.id),
  );
  const lateBlockingIssues = lateIssues
    .filter((issue) => ["BLOCKER", "MAJOR"].includes(issue.severity))
    .map((issue) => issue.id);
  const statusBlocks = verificationStatusBlocks(verification);
  const missingVerification = selection.roster.filter(
    (seat) => !verification.some((result) => result.seat === seat),
  );
  const allBlockingIssues = [...new Set([
    ...blockingIssues,
    ...lateBlockingIssues,
    ...statusBlocks.map((item) => item.issue_id),
  ])];
  const approved =
    allBlockingIssues.length === 0 &&
    verificationBlocks.length === 0 &&
    statusBlocks.length === 0 &&
    missingVerification.length === 0 &&
    verification.length > 0 &&
    verification.every((result) => result.signoff);
  return {
    artifact_kind: APPROVAL_ARTIFACT,
    schema_version: SELECTION_SCHEMA_VERSION,
    lifecycle_id: selection.lifecycle_id,
    phase: "verification",
    approved,
    blocking_issues: allBlockingIssues,
    response_blocking_issues: blockingIssues,
    late_blocking_issues: lateBlockingIssues,
    status_blocks: statusBlocks,
    verification_blocks: verificationBlocks,
    missing_verification_seats: missingVerification,
    signoff: approved,
    selection: selectionSummary(selection, table),
    current_candidate: candidateAddress(currentCandidate),
    selection_sha256: sha256(selection),
    discovery_ledger_sha256: sha256(ledger),
    response_sha256: null,
    verification_results_sha256: null,
    late_issue_ids: lateIssues.map((issue) => issue.id),
    late_blocker_count: lateIssues.filter((issue) => issue.severity === "BLOCKER").length,
    late_major_count: lateIssues.filter((issue) => issue.severity === "MAJOR").length,
    ledger: ledgerWithLate,
  };
}

export function validateApprovalArtifact(approval, options = {}) {
  if (!isPlainObject(approval)) error("approval artifact must be an object");
  const approvalKeys = [
    "artifact_kind",
    "schema_version",
    "lifecycle_id",
    "phase",
    "approved",
    "blocking_issues",
    "response_blocking_issues",
    "late_blocking_issues",
    "status_blocks",
    "verification_blocks",
    "missing_verification_seats",
    "signoff",
    "selection",
    "current_candidate",
    "selection_sha256",
    "discovery_ledger_sha256",
    "response_sha256",
    "verification_results_sha256",
    "late_issue_ids",
    "late_blocker_count",
    "late_major_count",
    "ledger",
  ];
  assertExactKeys(approval, approvalKeys, "approval artifact");
  for (const key of approvalKeys) {
    if (!Object.hasOwn(approval, key)) error(`approval artifact is missing ${key}`);
  }
  if (approval.artifact_kind !== APPROVAL_ARTIFACT) {
    error("approval artifact has an unexpected artifact_kind");
  }
  if (approval.schema_version !== SELECTION_SCHEMA_VERSION) {
    error("approval artifact schema_version is unsupported");
  }
  if (approval.phase !== "verification") error("approval artifact phase must be verification");
  if (typeof approval.approved !== "boolean" || approval.signoff !== approval.approved) {
    error("approval artifact approved and signoff must be matching booleans");
  }
  for (const key of [
    "blocking_issues",
    "response_blocking_issues",
    "late_blocking_issues",
    "missing_verification_seats",
    "late_issue_ids",
  ]) {
    if (!Array.isArray(approval[key])) error(`approval artifact ${key} must be an array`);
  }
  if (!Array.isArray(approval.status_blocks) || !Array.isArray(approval.verification_blocks)) {
    error("approval artifact blocking details must be arrays");
  }
  for (const key of ["late_blocker_count", "late_major_count"]) {
    if (!Number.isInteger(approval[key]) || approval[key] < 0) {
      error(`approval artifact ${key} must be a non-negative integer`);
    }
  }
  assertDigest(approval.discovery_ledger_sha256, "approval discovery_ledger_sha256");
  assertDigest(approval.selection_sha256, "approval selection_sha256");
  assertDigest(approval.response_sha256, "approval response_sha256");
  assertDigest(
    approval.verification_results_sha256,
    "approval verification_results_sha256",
  );
  validateLedger(approval.ledger);
  if (options.selection) {
    validateSelection(options.selection, options.table ?? readSelectionTable());
    if (approval.lifecycle_id !== options.selection.lifecycle_id) {
      error("approval artifact lifecycle_id disagrees with selection");
    }
    if (
      approval.selection?.candidate_id !== options.selection.candidate_id ||
      approval.selection?.snapshot_sha256 !== options.selection.snapshot_sha256
    ) {
      error("approval artifact selection disagrees with current selection");
    }
    const selectionDigest = options.selectionBytes
      ? sha256(options.selectionBytes)
      : sha256(options.selection);
    if (approval.selection_sha256 !== selectionDigest) {
      error("approval artifact is not bound to the current selection bytes");
    }
    validateSelectionCandidate(options.selection, approval.current_candidate);
  }
  if (options.ledgerBytes) {
    const actual = sha256(options.ledgerBytes);
    if (actual !== approval.discovery_ledger_sha256) {
      error("approval artifact is not bound to the immutable discovery ledger bytes");
    }
  }
  if (options.responseBytes) {
    const actual = sha256(options.responseBytes);
    if (actual !== approval.response_sha256) {
      error("approval artifact is not bound to the exact implementation response bytes");
    }
  }
  if (options.verificationResultsBytes) {
    const actual = sha256(options.verificationResultsBytes);
    if (actual !== approval.verification_results_sha256) {
      error("approval artifact is not bound to the exact adapted verification-result bytes");
    }
  }
  if (approval.approved && (
    approval.blocking_issues.length ||
    approval.verification_blocks.length ||
    approval.missing_verification_seats.length ||
    approval.status_blocks.length ||
    !approval.signoff
  )) {
    error("approved artifact carries blocking conditions");
  }
  return approval;
}

export function createApprovalArtifact(input, options = {}) {
  const ledgerBytes = input.discovery_ledger_bytes ?? input.ledger_bytes;
  const responseBytes = input.responses_bytes ?? input.response_bytes;
  const verificationResultsBytes =
    input.verification_results_bytes ?? input.verification_bytes;
  if (typeof ledgerBytes !== "string" || ledgerBytes.length === 0) {
    error("approval requires the exact immutable discovery ledger bytes");
  }
  if (typeof responseBytes !== "string" || responseBytes.length === 0) {
    error("approval requires the exact implementation response bytes");
  }
  if (
    typeof verificationResultsBytes !== "string" ||
    verificationResultsBytes.length === 0
  ) {
    error("approval requires the exact adapted verification-result bytes");
  }
  const selection = input.current_selection ?? input.selection;
  const ledger = input.discovery_ledger ?? input.ledger;
  const responses = input.responses;
  const verificationResults = input.verification_results;
  if (!selection || !ledger || responses === undefined || verificationResults === undefined) {
    error("approval requires selection, ledger, responses, and verification results");
  }
  try {
    if (stableStringify(JSON.parse(ledgerBytes)) !== stableStringify(ledger)) {
      error("approval ledger object disagrees with the exact ledger bytes");
    }
    if (stableStringify(JSON.parse(responseBytes)) !== stableStringify(responses)) {
      error("approval response object disagrees with the exact response bytes");
    }
    if (
      stableStringify(JSON.parse(verificationResultsBytes)) !==
      stableStringify(verificationResults)
    ) {
      error(
        "approval verification result object disagrees with the exact adapted verification-result bytes",
      );
    }
  } catch (cause) {
    error(`approval input bytes are not valid JSON: ${cause.message}`);
  }
  const table = options.table ?? input.table;
  const approval = evaluateApproval({
    ...input,
    current_selection: selection,
    discovery_ledger: ledger,
    responses,
    verification_results: verificationResults,
  });
  validateVerificationResultArtifact(verificationResults, {
    selection,
    table,
    ledger,
    ledger_bytes: ledgerBytes,
    selection_bytes: input.selection_bytes,
  });
  approval.selection_sha256 = typeof input.selection_bytes === "string"
    ? sha256(input.selection_bytes)
    : sha256(selection);
  approval.discovery_ledger_sha256 = sha256(ledgerBytes);
  approval.response_sha256 = sha256(responseBytes);
  approval.verification_results_sha256 = sha256(verificationResultsBytes);
  validateApprovalArtifact(approval, {
    selection,
    selectionBytes: input.selection_bytes,
    table,
    ledgerBytes,
    responseBytes,
    verificationResultsBytes,
  });
  return sortedObject(approval);
}

export function writeApprovalArtifact(path, input, options = {}) {
  return writeCreateOrCompare(path, createApprovalArtifact(input, options));
}

export function calculateMetrics(input) {
  const ledger = input.ledger;
  if (ledger) validateLedger(ledger);
  const issues = ledger?.issues ?? input.issues ?? [];
  const lateIssues = issues.filter((issue) => issue.late === true);
  const verificationValues = input.verification_results?.results ??
    input.verification_results?.verdicts ??
    input.verification_results ??
    [];
  const verificationList = Array.isArray(verificationValues)
    ? verificationValues
    : isPlainObject(verificationValues)
      ? Object.values(verificationValues)
      : [];
  const lateVerificationFindings = verificationList.flatMap((result) =>
    Array.isArray(result?.late_findings) ? result.late_findings : [],
  );
  const responseList = ledger && input.responses
    ? validateResponses(ledger, input.responses)
    : Array.isArray(input.responses)
      ? input.responses
      : isPlainObject(input.responses)
        ? Object.values(input.responses)
        : [];
  const fixed = issues.filter((issue) => {
    const response = responseList.find((candidate) => candidate.issue_id === issue.id);
    return response?.disposition === "Fixed";
  }).length;
  const implementationIterations = Number(
    input.implementation_history?.length ??
    input.implementation_iterations ??
    input.implementationIterations ??
    0,
  );
  if (!Number.isInteger(implementationIterations) || implementationIterations < 0) {
    error("implementation_iterations must be a non-negative integer");
  }
  const reviewIterations = Number(
    input.verification_history?.length ??
    input.review_history?.length ??
    input.review_iterations ??
    input.reviewIterations ??
    0,
  );
  if (!Number.isInteger(reviewIterations) || reviewIterations < 0) {
    error("review_iterations must be a non-negative integer");
  }
  const initialUnique = issues.filter((issue) => issue.late !== true).length;
  const lateUnique = lateIssues.length + lateVerificationFindings.length;
  const lateBlockers =
    lateIssues.filter((issue) => issue.severity === "BLOCKER").length +
    lateVerificationFindings.filter((finding) =>
      verdictSeverity(finding.severity, "late verification severity") === "BLOCKER",
    ).length;
  const lateMajors =
    lateIssues.filter((issue) => issue.severity === "MAJOR").length +
    lateVerificationFindings.filter((finding) =>
      verdictSeverity(finding.severity, "late verification severity") === "MAJOR",
    ).length;
  for (const [label, value] of [
    ["initial_unique_findings", initialUnique],
    ["late_unique_findings", lateUnique],
    ["late_blocker_count", lateBlockers],
    ["late_major_count", lateMajors],
  ]) {
    if (!Number.isInteger(value) || value < 0) {
      error(`${label} must be a non-negative integer`);
    }
  }
  return {
    initial_unique_findings: initialUnique,
    late_unique_findings: lateUnique,
    late_blocker_count: lateBlockers,
    late_major_count: lateMajors,
    review_iterations: reviewIterations,
    implementation_iterations: implementationIterations,
    average_fixed_issues_per_implementation_iteration:
      implementationIterations === 0 ? 0.0 : fixed / implementationIterations,
  };
}

export function createMetricsArtifact(input, options = {}) {
  const ledger = input.ledger;
  const selection = input.selection;
  if (!selection) {
    error("final metrics require the current lifecycle selection");
  }
  const ledgerBytes = input.ledger_bytes;
  const responseBytes = input.responses_bytes;
  const verificationResultsBytes = input.verification_results_bytes;
  if (typeof ledgerBytes !== "string" || ledgerBytes.length === 0) {
    error("final metrics require exact immutable discovery ledger bytes");
  }
  if (typeof responseBytes !== "string" || responseBytes.length === 0) {
    error("final metrics require exact implementation response bytes");
  }
  if (
    typeof verificationResultsBytes !== "string" ||
    verificationResultsBytes.length === 0
  ) {
    error("final metrics require exact adapted verification-result bytes");
  }
  validateLedger(ledger);
  const table = options.table ?? input.table;
  validateSelection(selection, table);
  const responses = input.responses;
  const verificationResults = input.verification_results;
  if (responses === undefined || verificationResults === undefined) {
    error("final metrics require ledger, responses, and verification artifacts");
  }
  try {
    if (stableStringify(JSON.parse(ledgerBytes)) !== stableStringify(ledger)) {
      error("metrics ledger object disagrees with exact ledger bytes");
    }
    if (stableStringify(JSON.parse(responseBytes)) !== stableStringify(responses)) {
      error("metrics response object disagrees with exact response bytes");
    }
    if (
      stableStringify(JSON.parse(verificationResultsBytes)) !==
      stableStringify(verificationResults)
    ) {
      error("metrics verification result object disagrees with exact artifact bytes");
    }
  } catch (cause) {
    error(`metrics input bytes are not valid JSON: ${cause.message}`);
  }
  validateResponses(ledger, responses);
  const verifiedResults = validateVerificationResultArtifact(verificationResults, {
    selection,
    table,
    ledger,
    ledger_bytes: ledgerBytes,
    selection_bytes: input.selection_bytes,
  });
  if (
    verifiedResults.some((result) => !result.signoff) ||
    verificationStatusBlocks(verifiedResults).length > 0 ||
    verifiedResults.some((result) =>
      result.late_findings.some((finding) =>
        ["BLOCKER", "MAJOR"].includes(
          verdictSeverity(finding.severity, "late verification severity"),
        ),
      ),
    )
  ) {
    error("final metrics require complete passing verification results");
  }
  const metrics = calculateMetrics({
    ...input,
    ledger,
    responses,
    verification_results: verificationResults,
  });
  const artifact = {
    artifact_kind: METRICS_ARTIFACT,
    schema_version: SELECTION_SCHEMA_VERSION,
    lifecycle_id: ledger.lifecycle_id,
    program: ledger.program,
    wave: ledger.wave,
    candidate_id: ledger.candidate_id,
    content_id: ledger.content_id,
    snapshot_sha256: ledger.snapshot_sha256,
    selection_sha256: typeof input.selection_bytes === "string"
      ? sha256(input.selection_bytes)
      : sha256(selection),
    discovery_ledger_sha256: sha256(ledgerBytes),
    response_sha256: sha256(responseBytes),
    verification_results_sha256: sha256(verificationResultsBytes),
    status: "complete",
    degraded: false,
    verification_complete: true,
    metrics,
  };
  return sortedObject(artifact);
}

export function writeMetricsArtifact(path, input, options = {}) {
  return writeCreateOrCompare(path, createMetricsArtifact(input, options));
}

function panelFormat(value, label) {
  if (!isPlainObject(value)) error(`${label} must be a JSON object`);
  if (!Object.hasOwn(value, "panel_format_version")) return "legacy";
  if (value.panel_format_version !== 1) {
    error(`${label} has malformed or unknown panel_format_version`);
  }
  return "current";
}

export function probePanelFormat(value, label = "panel artifact") {
  return panelFormat(value, label);
}

const LEGACY_REQUEST_KEYS = [
  "artifact_kind",
  "schema_version",
  "program",
  "wave",
  "candidate_id",
  "content_id",
  "snapshot_sha256",
  "provider",
  "model_version",
  "reasoning_effort",
  "roles",
  "record_artifact_kind",
  "record_schema_version",
  "record_files",
];
const LEGACY_RECORD_KEYS = [
  "artifact_kind",
  "schema_version",
  "role",
  "candidate_id",
  "content_id",
  "snapshot_sha256",
  "model_version",
  "provider",
  "reasoning_effort",
  "run_id",
  "receipt_locator",
  "output_sha256",
  "signoff",
  "recommendations",
];
const LEGACY_ATTESTATION_KEYS = ["roles", "records", "unanimous"];

function legacyBundle(input) {
  if (typeof input === "string") {
    const path = resolve(input);
    if (!existsSync(path)) error(`missing legacy round at ${path}`);
    if (statSync(path).isFile()) {
      const parsed = readJson(path, "legacy JSON");
      if (!isPlainObject(parsed) || !Array.isArray(parsed.records) && !isPlainObject(parsed.records)) {
        error("legacy JSON must contain a coherent request and records");
      }
      return legacyBundle(parsed);
    }
    const requestPath = ["panel-request.json", "request.json"]
      .map((name) => join(path, name))
      .find((candidate) => existsSync(candidate));
    const recordDir = join(path, "records");
    if (!existsSync(recordDir) || !statSync(recordDir).isDirectory()) {
      error("legacy round is missing its records directory");
    }
    const entries = readdirSync(recordDir, { withFileTypes: true });
    const jsonEntries = entries.filter((entry) => entry.isFile() && entry.name.endsWith(".json"));
    if (jsonEntries.length !== entries.length) {
      error("legacy records directory contains a non-JSON or non-regular entry");
    }
    const records = jsonEntries
      .sort((left, right) => left.name.localeCompare(right.name))
      .map((entry) => ({
        name: entry.name,
        record: readJson(join(recordDir, entry.name), `legacy record ${entry.name}`),
        bytes: readFileSync(join(recordDir, entry.name)),
      }));
    const attestationPath = join(path, "attestation.json");
    const sealPath = join(path, "seal.json");
    return {
      request: requestPath
        ? readJson(requestPath, "legacy panel request")
        : undefined,
      records,
      attestation: existsSync(attestationPath)
        ? readJson(attestationPath, "legacy panel attestation")
        : undefined,
      seal: existsSync(sealPath) ? readJson(sealPath, "legacy seal") : undefined,
      exactBytes: true,
    };
  }
  if (!isPlainObject(input)) {
    error("legacy input must be a coherent request and records object");
  }
  const rawRecords = input.records;
  if (!Array.isArray(rawRecords) && !isPlainObject(rawRecords)) {
    error("legacy input must contain a records array or object");
  }
  const suppliedRecordBytes = input.record_bytes ?? input.recordBytes;
  if (
    suppliedRecordBytes !== undefined &&
    (!isPlainObject(suppliedRecordBytes) ||
      Object.values(suppliedRecordBytes).some((bytes) => typeof bytes !== "string"))
  ) {
    error("legacy record_bytes must be an object of exact UTF-8 record strings");
  }
  const recordEntry = (record, role, name) => {
    const normalized = {
      ...(isPlainObject(record) ? record : {}),
      ...(role && !record?.role ? { role } : {}),
    };
    const recordRole = normalized.role;
    const exact = suppliedRecordBytes?.[recordRole];
    if (exact !== undefined) {
      try {
        if (stableStringify(JSON.parse(exact)) !== stableStringify(normalized)) {
          error(`legacy exact bytes for ${recordRole} disagree with the supplied record object`);
        }
      } catch (cause) {
        error(`legacy exact bytes for ${recordRole} are not valid JSON: ${cause.message}`);
      }
      return { name, record: normalized, bytes: Buffer.from(exact, "utf8") };
    }
    return {
      name,
      record: normalized,
      bytes: Buffer.from(stableStringify(normalized)),
    };
  };
  const records = Array.isArray(rawRecords)
    ? rawRecords.map((record) => recordEntry(record, record?.role, undefined))
    : Object.entries(rawRecords).map(([role, record]) =>
        recordEntry(record, role, `${role}.json`),
      );
  const exactBytes = records.every((entry) =>
    suppliedRecordBytes?.[entry.record.role] !== undefined,
  );
  return {
    request: input.request,
    records,
    attestation: input.attestation,
    seal: input.seal,
    seal_panel: input.seal_panel,
    exactBytes,
  };
}

function validateLegacyRequest(request) {
  if (!isPlainObject(request)) error("legacy panel request is required");
  if (Object.hasOwn(request, "panel_format_version")) {
    error("legacy panel request must omit panel_format_version");
  }
  assertExactKeys(request, LEGACY_REQUEST_KEYS, "legacy panel request");
  if (request.artifact_kind !== "d2b-delivery/panel-request") {
    error("legacy panel request has an unexpected artifact_kind");
  }
  if (request.schema_version !== 2 || request.record_schema_version !== 2) {
    error("legacy panel request schema_version must be 2");
  }
  if (request.record_artifact_kind !== "d2b-delivery/panel-receipt") {
    error("legacy panel request has an unexpected record_artifact_kind");
  }
  if (
    !Array.isArray(request.roles) ||
    !Array.isArray(request.record_files) ||
    request.roles.some((role) => typeof role !== "string") ||
    request.record_files.some((file) => typeof file !== "string")
  ) {
    error("legacy panel request roles and record_files must be arrays of strings");
  }
  if (request.roles.join(",") !== LEGACY_ROSTER.join(",")) {
    error("legacy panel request must retain the exact fixed-ten roster including rust");
  }
  const expectedFiles = LEGACY_ROSTER.map((role) => `${role}.json`);
  if (request.record_files.join(",") !== expectedFiles.join(",")) {
    error("legacy panel request record_files must follow the fixed-ten roster");
  }
  if (
    request.provider !== "github-copilot" ||
    request.model_version !== LEGACY_MODEL_POLICY ||
    request.reasoning_effort !== LEGACY_EFFORT_POLICY
  ) {
    error("legacy panel request must use the exact legacy provider/model/effort binding");
  }
  candidateAddress(request);
  return request;
}

function validateLegacyAttestation(attestation, recordsByRole, exactBytes = false) {
  if (attestation === undefined) return;
  if (Object.hasOwn(attestation, "panel_format_version")) {
    error("legacy panel attestation must omit panel_format_version");
  }
  assertExactKeys(attestation, LEGACY_ATTESTATION_KEYS, "legacy panel attestation");
  if (!Array.isArray(attestation.roles)) {
    error("legacy panel attestation roles must be an array");
  }
  if (attestation.roles.join(",") !== LEGACY_ROSTER.join(",")) {
    error("legacy panel attestation must retain the exact fixed-ten roster");
  }
  if (!Array.isArray(attestation.records) || attestation.records.length !== LEGACY_ROSTER.length) {
    error("legacy panel attestation must contain exactly ten records");
  }
  if (typeof attestation.unanimous !== "boolean") {
    error("legacy panel attestation unanimous must be boolean");
  }
  const seen = new Set();
  for (const item of attestation.records) {
    assertExactKeys(item, ["role", "file", "sha256", "run_id"], "legacy attested record");
    if (!LEGACY_ROSTER.includes(item.role) || seen.has(item.role)) {
      error("legacy panel attestation repeats or omits a fixed-ten role");
    }
    seen.add(item.role);
    assertDigest(item.sha256, "legacy attested record sha256");
    if (item.file !== `${item.role}.json`) {
      error("legacy panel attestation filename does not match its role");
    }
    const recordEntry = recordsByRole.get(item.role);
    if (!recordEntry || item.run_id !== recordEntry.record.run_id) {
      error(`legacy panel attestation provenance does not match ${item.role}`);
    }
    if (
      exactBytes &&
      item.sha256 !== createHash("sha256").update(recordEntry.bytes).digest("hex")
    ) {
      error(`legacy panel attestation digest does not match exact bytes for ${item.role}`);
    }
  }
}

function legacySeverity(text) {
  const prefixes = [
    ["critical", "BLOCKER"],
    ["high", "MAJOR"],
    ["medium", "MINOR"],
    ["low", "NIT"],
  ];
  for (const [prefix, severity] of prefixes) {
    const marker = `[${prefix}]`;
    if (
      typeof text === "string" &&
      text.length >= marker.length &&
      text.slice(0, marker.length).toLowerCase() === marker
    ) {
      return { severity, migration_assigned_severity: false };
    }
  }
  return { severity: "MAJOR", migration_assigned_severity: true };
}

export function importLegacyRound(input, options = {}) {
  const rawInput = typeof input === "string" ? undefined : input;
  if (rawInput && Object.hasOwn(rawInput, "panel_format_version")) {
    const format = panelFormat(rawInput, "legacy round");
    if (format !== "legacy") {
      error("legacy round is current format; refusing legacy fallback");
    }
  }
  const bundle = legacyBundle(input);
  const request = bundle.request;
  const completeEnvelope = request !== undefined;
  if (completeEnvelope) validateLegacyRequest(request);
  if (bundle.attestation !== undefined) {
    panelFormat(bundle.attestation, "legacy panel attestation");
  }
  const recordEntries = bundle.records;
  const ordered = [];
  const seenRoles = new Set();
  let commonAddress = request ? candidateAddress(request) : undefined;
  for (const [index, entry] of recordEntries.entries()) {
    const record = entry.record;
    const format = panelFormat(record, `legacy record ${index + 1}`);
    if (format !== "legacy") {
      error(`legacy record ${index + 1} is current format; refusing legacy fallback`);
    }
    if (!isPlainObject(record)) error(`legacy record ${index + 1} must be an object`);
    assertExactKeys(record, LEGACY_RECORD_KEYS, `legacy record ${index + 1}`);
    if (
      record.artifact_kind !== "d2b-delivery/panel-receipt" ||
      record.schema_version !== 2
    ) {
      error(`legacy record ${index + 1} has an unexpected artifact or schema`);
    }
    const role = nonBlank(record.role, `legacy record ${index + 1}.role`);
    if (!LEGACY_ROSTER.includes(role)) error(`legacy record has unknown role "${role}"`);
    if (seenRoles.has(role)) error(`legacy round has duplicate record for ${role}`);
    seenRoles.add(role);
    if (
      request &&
      entry.name === undefined &&
      request.roles[index] !== role
    ) {
      error(`legacy record ${index + 1} is out of fixed-ten request order`);
    }
    if (entry.name !== undefined && entry.name !== `${role}.json`) {
      error(`legacy record filename ${entry.name} does not match role ${role}`);
    }
    const address = {
      candidate_id: safePathPart(record.candidate_id, `legacy record ${role}.candidate_id`),
      content_id: nonBlank(record.content_id, `legacy record ${role}.content_id`),
      snapshot_sha256: assertDigest(
        record.snapshot_sha256,
        `legacy record ${role}.snapshot_sha256`,
      ),
    };
    if (!commonAddress) commonAddress = address;
    for (const key of ["candidate_id", "content_id", "snapshot_sha256"]) {
      if (address[key] !== commonAddress[key]) {
        error(`legacy record ${role} is not candidate-coherent`);
      }
    }
    for (const key of ["provider", "model_version", "reasoning_effort"]) {
      if (
        record[key] !==
        (key === "provider"
          ? "github-copilot"
          : key === "model_version"
            ? LEGACY_MODEL_POLICY
            : LEGACY_EFFORT_POLICY)
      ) {
        error(`legacy record ${role} has a non-legacy binding`);
      }
    }
    if (!Array.isArray(record.recommendations)) {
      error(`legacy record ${role}.recommendations must be an array`);
    }
    nonBlank(record.run_id, `legacy record ${role}.run_id`);
    nonBlank(record.receipt_locator, `legacy record ${role}.receipt_locator`);
    if (!record.receipt_locator.startsWith("github-copilot://")) {
      error(`legacy record ${role}.receipt_locator has the wrong provider scheme`);
    }
    if (typeof record.signoff !== "boolean" ||
        record.signoff !== (record.recommendations.length === 0)) {
      error(`legacy record ${role} has inconsistent signoff`);
    }
    assertDigest(record.output_sha256, `legacy record ${role}.output_sha256`);
    ordered.push({
      role,
      record,
      digest: createHash("sha256").update(entry.bytes).digest("hex"),
      bytes: entry.bytes,
    });
  }
  const recordsByRole = new Map(ordered.map((entry) => [entry.role, entry]));
  if (request) {
    for (const key of ["candidate_id", "content_id", "snapshot_sha256"]) {
      if (commonAddress?.[key] !== request[key]) {
        error(`legacy records disagree with the panel request ${key}`);
      }
    }
    const recordsByRole = new Map(ordered.map((entry) => [entry.role, entry]));
    const runIds = new Set();
    const receipts = new Set();
    for (const entry of ordered) {
      if (runIds.has(entry.record.run_id)) {
        error(`legacy records repeat run_id ${entry.record.run_id}`);
      }
      if (receipts.has(entry.record.receipt_locator)) {
        error(`legacy records repeat receipt_locator ${entry.record.receipt_locator}`);
      }
      runIds.add(entry.record.run_id);
      receipts.add(entry.record.receipt_locator);
    }
  }
  /*
   * A missing proof means the publication is incomplete, not malformed.
   * When a proof is supplied, however, validate it even for a partial
   * publication so a broken attestation cannot be smuggled through as a
   * request to run current discovery.
   */
  validateLegacyAttestation(bundle.attestation, recordsByRole, bundle.exactBytes);
  if (
    bundle.attestation?.unanimous === true &&
    ordered.some((entry) => entry.record.signoff !== true)
  ) {
    error("legacy unanimous attestation contains a finding record");
  }
  const sealPanel =
    bundle.seal?.seal_panel ??
    bundle.seal?.panel ??
    bundle.seal_panel;
  if (sealPanel !== undefined) {
    validateLegacyAttestation(sealPanel, recordsByRole, bundle.exactBytes);
  }
  ordered.sort(
    (left, right) =>
      LEGACY_ROSTER.indexOf(left.role) - LEGACY_ROSTER.indexOf(right.role),
  );
  const sources = [];
  const responsibilities = [];
  for (const { role, record, digest } of ordered) {
    const recordAttribution = record.attribution ?? record.raw_attribution ?? role;
    nonBlank(recordAttribution, `legacy record ${role}.attribution`);
    for (const [index, recommendation] of record.recommendations.entries()) {
      if (typeof recommendation !== "string") {
        error(`legacy ${role} recommendation ${index + 1} must remain raw text`);
      }
      const mapped = legacySeverity(recommendation);
      const sourceId = `legacy:${digest}:${role}:${index + 1}`;
      const source = {
        source_id: sourceId,
        seat: role,
        source_ordinal: index + 1,
        raw_text: recommendation,
        raw_attribution: recordAttribution,
        attribution: recordAttribution,
        severity: mapped.severity,
        impact: "Imported legacy recommendation; impact requires current verification.",
        recommendation,
        ...(mapped.migration_assigned_severity
          ? { migration_assigned_severity: true }
          : {}),
      };
      sources.push(source);
    }
    if (role === "rust") {
      responsibilities.push({
        legacy_seat: "rust",
        current_seat: "software",
        profile: "rust",
      });
    }
  }
  const suppliedCandidate = options.candidate ?? rawInput?.candidate;
  const candidate = suppliedCandidate ?? (request ? candidateAddress(request) : undefined);
  if (suppliedCandidate && commonAddress) {
    const suppliedAddress = candidateAddress(suppliedCandidate);
    for (const key of ["candidate_id", "content_id", "snapshot_sha256"]) {
      if (suppliedAddress[key] !== commonAddress[key]) {
        error(`legacy candidate disagrees with records ${key}`);
      }
    }
  }
  if (suppliedCandidate && request) {
    validateSelectionCandidate(
      {
        ...candidateAddress(request),
        lifecycle_id: "legacy",
      },
      suppliedCandidate,
    );
  }
  let currentRoster;
  let currentSelection;
  let currentProfiles = {};
  const table = options.table ?? readSelectionTable(options.table_path);
  if (options.selection) {
    currentSelection =
      typeof options.selection === "string"
        ? readSelection(options.selection, { table })
        : options.selection;
    validateSelection(currentSelection, table);
    if (candidate) validateSelectionCandidate(currentSelection, candidate);
    currentRoster = currentSelection.roster;
    currentProfiles = currentSelection.profiles;
    if (candidate) {
      const fresh = selectRoster(candidate, { table });
      currentRoster = unionRosters([currentRoster, fresh.roster], table);
      currentProfiles = Object.fromEntries(
        currentRoster.map((seat) => [
          seat,
          [...new Set([
            ...(currentProfiles[seat] ?? []),
            ...(fresh.profiles[seat] ?? []),
          ])].sort(),
        ]),
      );
    }
  } else if (candidate) {
    const plan = selectRoster(candidate, { table });
    currentRoster = plan.roster;
    currentProfiles = plan.profiles;
  } else {
    currentRoster = table.mandatory_seats;
  }
  const importedCurrentSeats = [...new Set(
    ordered
      .map(({ role }) => (role === "rust" ? "software" : role))
      .filter((role) => table.mandatory_seats.includes(role) || table.optional_seats.includes(role)),
  )];
  const roster = unionRosters(
    [currentRoster, [...new Set([...table.mandatory_seats, ...importedCurrentSeats])]],
    table,
  );
  const profiles = Object.fromEntries(
    roster.map((seat) => [
      seat,
      [...new Set([
        ...(currentProfiles[seat] ?? []),
        ...(ordered.some(({ role }) => role === "rust") && seat === "software"
          ? ["rust"]
          : []),
      ])].sort(),
    ]),
  );
  const complete =
    completeEnvelope &&
    bundle.exactBytes &&
    bundle.attestation !== undefined &&
    LEGACY_ROSTER.every((role) => seenRoles.has(role));
  const groups = sources.map((source) => ({
    id: `R${sources.indexOf(source) + 1}`,
    source_finding_ids: [source.source_id],
    description: source.raw_text,
    severity: source.severity,
    impact: source.impact,
    recommendation: source.recommendation,
    late: false,
  }));
  return sortedObject({
    artifact_kind: LEGACY_IMPORT_ARTIFACT,
    schema_version: SELECTION_SCHEMA_VERSION,
    format: "legacy",
    legacy_roster: [...LEGACY_ROSTER],
    completed_legacy_seats: ordered.map(({ role }) => role),
    complete,
    discovery_input: true,
    discovery_required: !complete,
    discovery_mode: complete ? "use-imported-discovery" : "run-one-current-discovery",
    lifecycle_roster: roster,
    profiles,
    responsibilities,
    sources,
    dedup_groups: groups,
    ...(currentSelection ? { selection: currentSelection } : {}),
    ...(candidate ? { candidate: candidateAddress(candidate) } : {}),
  });
}

function readOptionalJson(path) {
  return path ? readJson(path) : undefined;
}

function flagValue(argv, name) {
  const index = argv.indexOf(name);
  if (index === -1) return undefined;
  if (!argv[index + 1] || argv[index + 1].startsWith("--")) {
    error(`${name} requires a value`);
  }
  return argv[index + 1];
}

function readJsonDirectory(path, label) {
  if (!existsSync(path) || !statSync(path).isDirectory()) {
    return readJson(path, label);
  }
  const entries = readdirSync(path, { withFileTypes: true })
    .sort((left, right) => left.name.localeCompare(right.name));
  if (entries.some((entry) => !entry.isFile() || !entry.name.endsWith(".json"))) {
    error(`${label} directory must contain only regular JSON files`);
  }
  if (entries.length === 0) error(`${label} directory contains no JSON artifacts`);
  return Object.fromEntries(
    entries.map((entry) => [
      entry.name.slice(0, -5),
      readJson(join(path, entry.name), `${label}/${entry.name}`),
    ]),
  );
}

function readVerificationVerdicts(path, selection) {
  const isDirectory = existsSync(path) && statSync(path).isDirectory();
  const verdicts = readJsonDirectory(path, "actual verification verdicts");
  if (!isDirectory) return verdicts;
  const expected = new Set(selection.roster);
  const actual = Object.keys(verdicts);
  for (const seat of actual) {
    if (!expected.has(seat)) {
      error(
        `actual verification verdict directory contains unselected seat "${seat}"`,
      );
    }
    const declared = verdicts[seat]?.engineer ?? verdicts[seat]?.seat;
    if (declared !== seat) {
      error(
        `actual verification verdict ${seat}.json declares seat "${declared}"; ` +
        "filename and selected seat must agree",
      );
    }
  }
  const missing = selection.roster.filter((seat) => !actual.includes(seat));
  if (missing.length > 0) {
    error(
      `actual verification verdict directory is missing selected seat(s): ${missing.join(", ")}`,
    );
  }
  return verdicts;
}

function usage() {
  return [
    "usage:",
    "  panel-lifecycle.mjs select <candidate.json> <lifecycle-id> [--phase discovery|verification] [--previous-selection PATH] [--fix-delta PATH] [--git-range RANGE]",
    "  panel-lifecycle.mjs discovery-request <selection.json> <candidate.json> <output.json>",
    "  panel-lifecycle.mjs adapt-discovery <verdicts.json> <output.json>",
    "  panel-lifecycle.mjs merge-ledger <selection.json> <results.json> <groups.json> <output.json>",
    "  panel-lifecycle.mjs response-template <ledger.json> <output.json>",
    "  panel-lifecycle.mjs adapt-verification <ledger.json> <verdicts.json|verdicts-dir> <output.json> --selection PATH --candidate PATH",
    "  panel-lifecycle.mjs verification <selection.json> <ledger.json> <responses.json> <self-verification.json> <output-dir> --candidate PATH --prior-selection PATH --prior-verdicts DIR --delta PATH",
    "  panel-lifecycle.mjs approval <selection.json> <ledger.json> <responses.json> <verification-results.json> <output.json> --candidate PATH",
    "  panel-lifecycle.mjs metrics --selection PATH --ledger PATH --responses PATH --verification-results PATH --output PATH [--implementation-history PATH] [--verification-history PATH]",
    "  panel-lifecycle.mjs import-legacy <legacy-dir-or-json> [candidate.json] <output.json>",
    "  panel-lifecycle.mjs validate-selection <selection.json>",
  ].join("\n");
}

async function main(argv) {
  const command = argv[0];
  if (!command) {
    console.error(usage());
    process.exitCode = 2;
    return;
  }
  try {
    if (command === "select") {
      const candidatePath = argv[1];
      const lifecycleId = argv[2];
      if (!candidatePath || !lifecycleId) error(usage());
      const phaseIndex = argv.indexOf("--phase");
      const phase = phaseIndex === -1 ? "discovery" : argv[phaseIndex + 1];
      if (!["discovery", "verification"].includes(phase)) {
        error("--phase must be discovery or verification");
      }
      const candidate = readJson(candidatePath, "candidate address");
      const gitRange = flagValue(argv, "--git-range");
      if (gitRange) {
        candidate.changed_paths = changedPathsFromGitRange(gitRange);
      }
      const previousPath =
        flagValue(argv, "--previous-selection") ??
        flagValue(argv, "--prior-selection");
      const previousSelection = previousPath
        ? readSelection(previousPath)
        : undefined;
      const fixDeltaPath = flagValue(argv, "--fix-delta");
      const fixDelta = fixDeltaPath
        ? readJson(fixDeltaPath, "fix delta")
        : undefined;
      if (phase === "discovery" && (previousPath || fixDeltaPath)) {
        error("prior selection and fix delta are verification-only selection inputs");
      }
      if (phase === "verification" && (!previousPath || !fixDeltaPath)) {
        error(
          "verification selection requires --previous-selection and --fix-delta",
        );
      }
      const result = createSelection({
        ...candidate,
        lifecycle_id: lifecycleId,
        phase,
        ...(phase === "verification"
          ? {
              full_candidate: candidate,
              ...(fixDelta ? { fix_delta: fixDelta } : {}),
              ...(previousSelection ? { previous_selection: previousSelection } : {}),
            }
          : {}),
      });
      console.log(result.path);
      return;
    }
    if (command === "validate-selection") {
      const selection = readSelection(argv[1]);
      console.log(stableStringify(selection));
      return;
    }
    if (command === "discovery-request") {
      const selection = readSelection(argv[1]);
      const candidate = readJson(argv[2], "candidate address");
      const request = createDiscoveryRequest({ selection, candidate });
      writeCreateOrCompare(argv[3], request);
      console.log(argv[3]);
      return;
    }
    if (command === "adapt-discovery") {
      const verdicts = readJson(argv[1], "actual discovery verdicts");
      writeCreateOrCompare(argv[2], {
        artifact_kind: DISCOVERY_RESULT_ARTIFACT,
        schema_version: SELECTION_SCHEMA_VERSION,
        results: adaptDiscoveryResults(verdicts),
      });
      console.log(argv[2]);
      return;
    }
    if (command === "merge-ledger") {
      const selection = readSelection(argv[1]);
      const results = readJson(argv[2], "discovery results");
      const groups = readJson(argv[3], "deduplication groups");
      const ledger = mergeDiscoveryLedger({
        selection,
        results: results.results ?? results.verdicts ?? results,
        groups,
      });
      writeCreateOrCompare(argv[4], ledger);
      console.log(argv[4]);
      return;
    }
    if (command === "response-template") {
      const ledger = readJson(argv[1], "issue ledger");
      writeResponseTemplate(argv[2], ledger);
      console.log(argv[2]);
      return;
    }
    if (command === "verification") {
      const selection = readSelection(argv[1]);
      const ledger = readJson(argv[2], "issue ledger");
      const responses = readJson(argv[3], "implementation responses");
      const selfVerification = readJson(argv[4], "self-verification");
      const optionsArgv = argv.slice(6);
      const candidatePath = flagValue(optionsArgv, "--candidate");
      const priorPath =
        flagValue(optionsArgv, "--prior-selection") ??
        flagValue(optionsArgv, "--previous-selection");
      const priorVerdictsPath = flagValue(optionsArgv, "--prior-verdicts");
      const deltaPath = flagValue(optionsArgv, "--delta");
      if (!candidatePath || !priorPath || !priorVerdictsPath || !deltaPath) {
        error(
          "verification requires --candidate, --prior-selection, --prior-verdicts, and --delta",
        );
      }
      const delta = readJson(deltaPath, "actual fix delta");
      const actualDeltaPaths = Array.isArray(delta)
        ? delta
        : delta?.changed_paths ?? delta?.paths ?? [];
      const result = writeVerificationArtifacts(argv[5], {
        current_selection: selection,
        discovery_ledger: ledger,
        responses,
        self_verification: selfVerification,
        current_candidate: readJson(candidatePath, "current candidate"),
        prior_selection: readSelection(priorPath),
        prior_verdicts: readJsonDirectory(priorVerdictsPath, "prior verdicts"),
        actual_delta_paths: actualDeltaPaths,
      });
      console.log(`wrote ${result.written.length} verification artifacts to ${argv[5]}`);
      return;
    }
    if (command === "adapt-verification") {
      const ledgerPath = argv[1];
      const ledgerBytes = readFileSync(ledgerPath, "utf8");
      const ledger = JSON.parse(ledgerBytes);
      validateLedger(ledger);
      const optionsArgv = argv.slice(4);
      const selectionPath = flagValue(optionsArgv, "--selection");
      const candidatePath = flagValue(optionsArgv, "--candidate");
      if (!selectionPath || !candidatePath) {
        error("adapt-verification requires --selection and --candidate");
      }
      const selectionBytes = readFileSync(selectionPath, "utf8");
      const selection = readSelection(selectionPath);
      const verdicts = readVerificationVerdicts(argv[2], selection);
      const artifact = createVerificationResultArtifact({
        selection,
        selection_bytes: selectionBytes,
        ledger,
        ledger_bytes: ledgerBytes,
        current_candidate: readJson(candidatePath, "current candidate"),
        results: verdicts,
      });
      writeCreateOrCompare(argv[3], artifact);
      console.log(argv[3]);
      return;
    }
    if (command === "approval" || command === "approve") {
      const selection = readSelection(argv[1]);
      const selectionBytes = readFileSync(argv[1], "utf8");
      const ledgerPath = argv[2];
      const ledgerBytes = readFileSync(ledgerPath, "utf8");
      const ledger = JSON.parse(ledgerBytes);
      const responseBytes = readFileSync(argv[3], "utf8");
      const verificationResultsBytes = readFileSync(argv[4], "utf8");
      const responses = JSON.parse(responseBytes);
      const verificationResults = JSON.parse(verificationResultsBytes);
      const candidatePath = flagValue(argv.slice(6), "--candidate");
      if (!candidatePath) error("approval requires --candidate");
      const currentCandidate = readJson(candidatePath, "current candidate");
      const approval = createApprovalArtifact({
        current_selection: selection,
        selection_bytes: selectionBytes,
        discovery_ledger: ledger,
        discovery_ledger_bytes: ledgerBytes,
        current_candidate: currentCandidate,
        responses,
        responses_bytes: responseBytes,
        verification_results: verificationResults,
        verification_results_bytes: verificationResultsBytes,
      });
      writeCreateOrCompare(argv[5], sortedObject(approval));
      console.log(argv[5]);
      if (!approval.approved) process.exitCode = 3;
      return;
    }
    if (command === "metrics") {
      const selectionPath = flagValue(argv, "--selection");
      const ledgerPath = flagValue(argv, "--ledger");
      const responsesPath = flagValue(argv, "--responses");
      const verificationPath = flagValue(argv, "--verification-results");
      const outputPath = flagValue(argv, "--output");
      if (!selectionPath || !ledgerPath || !responsesPath || !verificationPath || !outputPath) {
        error(usage());
      }
      const selectionBytes = readFileSync(selectionPath, "utf8");
      const selection = readSelection(selectionPath);
      const ledgerBytes = readFileSync(ledgerPath, "utf8");
      const ledger = JSON.parse(ledgerBytes);
      const responseBytes = readFileSync(responsesPath, "utf8");
      const verificationResultsBytes = readFileSync(verificationPath, "utf8");
      const responses = JSON.parse(responseBytes);
      const verificationResults = JSON.parse(verificationResultsBytes);
      const optionsArgv = argv;
      const implementationHistoryPath = flagValue(optionsArgv, "--implementation-history");
      const verificationHistoryPath = flagValue(optionsArgv, "--verification-history");
      const artifact = createMetricsArtifact({
        selection,
        selection_bytes: selectionBytes,
        ledger,
        ledger_bytes: ledgerBytes,
        responses,
        responses_bytes: responseBytes,
        verification_results: verificationResults,
        verification_results_bytes: verificationResultsBytes,
        implementation_history: implementationHistoryPath
          ? readJson(implementationHistoryPath, "implementation history")
          : [responses],
        verification_history: verificationHistoryPath
          ? readJson(verificationHistoryPath, "verification history")
          : verificationResults
            ? [verificationResults]
            : [],
      });
      writeCreateOrCompare(outputPath, artifact);
      console.log(outputPath);
      return;
    }
    if (command === "import-legacy") {
      const candidatePath = argv.length >= 4 ? argv[2] : undefined;
      const outputPath = argv.length >= 4 ? argv[3] : argv[2];
      const candidate = readOptionalJson(candidatePath);
      const imported = importLegacyRound(argv[1], { candidate });
      writeCreateOrCompare(outputPath, imported);
      console.log(outputPath);
      return;
    }
    error(`unknown command "${command}"\n${usage()}`);
  } catch (cause) {
    console.error(`error: ${cause.message}`);
    process.exitCode = 1;
  }
}

const entry = process.argv[1] ? pathToFileURL(resolve(process.argv[1])).href : "";
if (entry === import.meta.url) {
  await main(process.argv.slice(2));
}
