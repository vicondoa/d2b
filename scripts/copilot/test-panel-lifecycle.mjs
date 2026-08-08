#!/usr/bin/env node
// Focused behavior coverage for the standard Copilot panel lifecycle.

import {
  appendLateFindings,
  adaptDiscoveryVerdict,
  adaptVerificationVerdict,
  calculateMetrics,
  changedPathsFromGitRange,
  createApprovalArtifact,
  createDiscoveryRequest,
  createResponseTemplate,
  createSelection,
  evaluateApproval,
  importLegacyRound,
  lateFindingAdmission,
  mergeDiscoveryLedger,
  prepareVerification,
  readSelection,
  readSelectionTable,
  selectRoster,
  selectLifecycleRoster,
  sha256,
  stableStringify,
  validateDiscoveryResults,
  validateCandidateAgainstSelection,
  validateFixScope,
  validateMonotonicRoster,
  validateResponses,
  validateSelection,
  validateSelfVerification,
  validateVerificationRequest,
  validateVerificationResults,
  writeDirectoryCreateOrCompare,
  writeVerificationArtifacts,
  writeCreateOrCompare,
} from "../../.github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs";
import {
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { execFileSync, spawn, spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const LIFECYCLE_CLI = join(
  fileURLToPath(new URL(".", import.meta.url)),
  "..",
  "..",
  ".github",
  "skills",
  "d2b-panel-round",
  "scripts",
  "panel-lifecycle.mjs",
);
const REPOSITORY_ROOT = join(
  fileURLToPath(new URL(".", import.meta.url)),
  "..",
  "..",
);

let failures = 0;
const check = (name, ok, detail = "") => {
  if (ok) {
    console.log(`  ok   ${name}`);
  } else {
    failures += 1;
    console.error(`  FAIL ${name}${detail ? `: ${detail}` : ""}`);
  }
};

function rejects(name, fn, pattern) {
  try {
    fn();
    check(name, false, "accepted invalid input");
  } catch (cause) {
    check(name, pattern.test(cause.message), cause.message);
  }
}

function concurrentDirectoryPublish(directory, helperPath) {
  const source = `
import { pathToFileURL } from "node:url";
const [helperPath, directory, bytes] = process.argv.slice(1);
try {
  const entry = process.argv[1];
  process.argv[1] = "";
  const { writeDirectoryCreateOrCompare } =
    await import(pathToFileURL(helperPath).href);
  process.argv[1] = entry;
  const result = writeDirectoryCreateOrCompare(directory, [
    { name: "seat.json", bytes },
  ]);
  console.log(JSON.stringify(result));
} catch (cause) {
  console.error(cause.message);
  process.exitCode = 1;
}
`;
  return new Promise((resolve) => {
    const results = [];
    const finish = () => {
      if (results.length === 2) resolve(results.sort((left, right) => left.index - right.index));
    };
    for (const [index, bytes] of ["first\n", "second\n"].entries()) {
      const child = spawn(
        process.execPath,
        ["--input-type=module", "-e", source, helperPath, directory, bytes],
        { encoding: "utf8" },
      );
      let stdout = "";
      let stderr = "";
      child.stdout.on("data", (chunk) => {
        stdout += chunk;
      });
      child.stderr.on("data", (chunk) => {
        stderr += chunk;
      });
      child.on("error", (cause) => {
        results.push({ index, status: 1, stdout, stderr: cause.message });
        finish();
      });
      child.on("close", (status) => {
        results.push({ index, status, stdout, stderr });
        finish();
      });
    }
  });
}

function observeDirectoryPublish(directory, helperPath) {
  const source = `
import { pathToFileURL } from "node:url";
const [helperPath, directory] = process.argv.slice(1);
try {
  const entry = process.argv[1];
  process.argv[1] = "";
  const { writeDirectoryCreateOrCompare } =
    await import(pathToFileURL(helperPath).href);
  process.argv[1] = entry;
  const bytes = "x".repeat(8 * 1024 * 1024);
  writeDirectoryCreateOrCompare(directory, [
    { name: "first.json", bytes },
    { name: "second.json", bytes },
  ]);
} catch (cause) {
  console.error(cause.message);
  process.exitCode = 1;
}
`;
  return new Promise((resolve) => {
    const child = spawn(
      process.execPath,
      ["--input-type=module", "-e", source, helperPath, directory],
      { encoding: "utf8" },
    );
    const expectedSize = 8 * 1024 * 1024;
    let violation = "";
    let stderr = "";
    const observe = () => {
      if (!existsSync(directory)) return;
      try {
        const entries = readdirSync(directory, { withFileTypes: true })
          .sort((left, right) => left.name.localeCompare(right.name));
        if (
          entries.length !== 2 ||
          entries.some((entry, index) =>
            !entry.isFile() ||
            entry.name !== ["first.json", "second.json"][index] ||
            readFileSync(join(directory, entry.name)).length !== expectedSize
          )
        ) {
          violation = "observer saw a partial published directory";
        }
      } catch (cause) {
        violation = `observer could not inspect published directory: ${cause.message}`;
      }
    };
    const timer = setInterval(observe, 1);
    child.stderr.on("data", (chunk) => {
      stderr += chunk;
    });
    child.on("close", (status) => {
      clearInterval(timer);
      observe();
      resolve({ status, stderr, violation });
    });
  });
}

function compareExistingDirectory(directory, helperPath, bytes) {
  const source = `
import { pathToFileURL } from "node:url";
const [helperPath, directory, bytes] = process.argv.slice(1);
try {
  const entry = process.argv[1];
  process.argv[1] = "";
  const { writeDirectoryCreateOrCompare } =
    await import(pathToFileURL(helperPath).href);
  process.argv[1] = entry;
  const result = writeDirectoryCreateOrCompare(directory, [
    { name: "seat.json", bytes },
  ]);
  console.log(JSON.stringify(result));
} catch (cause) {
  console.error(cause.message);
  process.exitCode = 1;
}
`;
  return new Promise((resolve) => {
    const child = spawn(
      process.execPath,
      ["--input-type=module", "-e", source, helperPath, directory, bytes],
      { encoding: "utf8" },
    );
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (chunk) => {
      stdout += chunk;
    });
    child.stderr.on("data", (chunk) => {
      stderr += chunk;
    });
    child.on("close", (status) => resolve({ status, stdout, stderr }));
  });
}

function unavailableDirectoryPublish(directory, helperPath, pathWithoutMv) {
  const source = `
import { pathToFileURL } from "node:url";
const [helperPath, directory] = process.argv.slice(1);
try {
  const entry = process.argv[1];
  process.argv[1] = "";
  const { writeDirectoryCreateOrCompare } =
    await import(pathToFileURL(helperPath).href);
  process.argv[1] = entry;
  const result = writeDirectoryCreateOrCompare(directory, [
    { name: "seat.json", bytes: "fault\\n" },
  ]);
  console.log(JSON.stringify(result));
} catch (cause) {
  console.error(cause.message);
  process.exitCode = 1;
}
`;
  return new Promise((resolve) => {
    const child = spawn(
      process.execPath,
      ["--input-type=module", "-e", source, helperPath, directory],
      {
        encoding: "utf8",
        env: { ...process.env, PATH: pathWithoutMv },
      },
    );
    let stderr = "";
    child.stderr.on("data", (chunk) => {
      stderr += chunk;
    });
    child.on("close", (status) => resolve({ status, stderr }));
  });
}

console.log("panel lifecycle: documented handoff");
const documentedHandoffLiterals = [
  'adapt-verification "$ROUND/discovery-ledger.json" "$ROUND/verdicts"',
  '--selection "$ROUND/selection.json" --candidate "$ROUND/current-candidate.json"',
  '--ledger "$ROUND/discovery-ledger.json" --responses "$ROUND/responses.json"',
  '--verification-results "$ROUND/verification-results.json"',
  '--approval "$ROUND/approval.json"',
];
for (const documentedPath of [
  join(REPOSITORY_ROOT, ".github", "skills", "d2b-panel-round", "SKILL.md"),
  join(REPOSITORY_ROOT, "docs", "contributing", "copilot-agents.md"),
]) {
  const documented = readFileSync(documentedPath, "utf8");
  for (const literal of documentedHandoffLiterals) {
    check(
      `${documentedPath.split("/").at(-1)} carries literal handoff path ${literal}`,
      documented.includes(literal),
    );
  }
}

function candidate(overrides = {}) {
  return {
    program: "SPEC004",
    wave: "spec004w1",
    candidate_id: "candidate-1",
    content_id: "content-1",
    snapshot_sha256: "a".repeat(64),
    changed_paths: ["src/panel.js"],
    ...overrides,
  };
}

function completeResults(roster, findingSeat = null) {
  return Object.fromEntries(
    roster.map((seat) => [
      seat,
      {
        seat,
        complete: true,
        findings: seat === findingSeat
          ? [{
              source_id: `${seat}:1`,
              source_ordinal: 1,
              raw_text: "The generated request can lose a source.",
              attribution: seat,
              severity: "MAJOR",
              impact: "A source could disappear from the shared ledger.",
              recommendation: "Validate the source mapping before generation.",
            }]
          : [],
      },
    ]),
  );
}

function makeSelection(root, overrides = {}) {
  return createSelection(
    {
      ...candidate(overrides),
      lifecycle_id: "spec004w1",
      phase: overrides.phase ?? "discovery",
    },
    { root },
  );
}

function makeLedger() {
  return {
    artifact_kind: "d2b-panel/issue-ledger",
    schema_version: 1,
    lifecycle_id: "spec004w1",
    selection_schema_version: 1,
    selection_table_version: 2,
    program: "SPEC004",
    wave: "spec004w1",
    candidate_id: "candidate-1",
    content_id: "content-1",
    snapshot_sha256: "a".repeat(64),
    roster: [
      "software",
      "test",
      "product",
      "docs",
      "security",
      "observability",
      "simplicity",
      "reliability",
      "agentic",
      "nixos",
    ],
    sources: [
      {
        source_id: "software:1",
        seat: "software",
        source_ordinal: 1,
        raw_text: "blocker",
        attribution: "software",
        severity: "BLOCKER",
        impact: "unsafe",
        recommendation: "fix",
      },
      {
        source_id: "test:1",
        seat: "test",
        source_ordinal: 1,
        raw_text: "major",
        attribution: "test",
        severity: "MAJOR",
        impact: "wrong",
        recommendation: "verify",
      },
      {
        source_id: "docs:1",
        seat: "docs",
        source_ordinal: 1,
        raw_text: "minor",
        attribution: "docs",
        severity: "MINOR",
        impact: "small",
        recommendation: "clean up",
      },
      {
        source_id: "product:1",
        seat: "product",
        source_ordinal: 1,
        raw_text: "nit",
        attribution: "product",
        severity: "NIT",
        impact: "small",
        recommendation: "clarify",
      },
    ],
    issues: [
      {
        id: "R1",
        description: "blocker",
        severity: "BLOCKER",
        impact: "unsafe",
        recommendation: "fix",
        source_finding_ids: ["software:1"],
        late: false,
      },
      {
        id: "R2",
        description: "major",
        severity: "MAJOR",
        impact: "wrong",
        recommendation: "verify",
        source_finding_ids: ["test:1"],
        late: false,
      },
      {
        id: "R3",
        description: "minor",
        severity: "MINOR",
        impact: "small",
        recommendation: "clean up",
        source_finding_ids: ["docs:1"],
        late: false,
      },
      {
        id: "R4",
        description: "nit",
        severity: "NIT",
        impact: "small",
        recommendation: "clarify",
        source_finding_ids: ["product:1"],
        late: false,
      },
    ],
    complete: true,
  };
}

function allVerificationResults(roster, lateFindings = {}, ledger = makeLedger()) {
  const statuses = Object.fromEntries(
    ledger.issues.map((issue) => [issue.id, "resolved"]),
  );
  return Object.fromEntries(
    roster.map((seat) => [
      seat,
      {
        seat,
        complete: true,
        signoff: true,
        summary: "Verified.",
        recommendations: [],
        verified_issue_statuses: statuses,
        late_findings: lateFindings[seat] ?? [],
      },
    ]),
  );
}

function adaptedVerificationArtifact(selection, ledger, results) {
  const selectionBytes = stableStringify(selection);
  const ledgerBytes = stableStringify(ledger);
  return {
    artifact_kind: "d2b-panel/verification",
    schema_version: 1,
    phase: "verification",
    lifecycle_id: selection.lifecycle_id,
    selection_sha256: sha256(selectionBytes),
    current_candidate: candidate({
      candidate_id: selection.candidate_id,
      content_id: selection.content_id,
      snapshot_sha256: selection.snapshot_sha256,
      changed_paths: selection.classification_inputs.changed_paths,
    }),
    discovery_ledger_sha256: sha256(ledgerBytes),
    results,
  };
}

console.log("panel lifecycle: selection table");
{
  const code = selectRoster(candidate());
  check("code floor selects ten seats", code.roster.length === 10);
  check(
    "mandatory seats are present",
    code.mandatory_seats.every((seat) => code.roster.includes(seat)),
  );
  check(
    "code floor uses deterministic fill order",
    code.floor_filled.join(",") === "reliability,agentic,nixos",
  );

  const documentation = selectRoster(
    candidate({ changed_paths: ["docs/guide.md"] }),
  );
  check("documentation floor selects eight seats", documentation.roster.length === 8);

  const triggerPaths = {
    reliability: ["src/restart-handler.js"],
    agentic: [".github/skills/example/SKILL.md"],
    nixos: ["nixos-modules/example.nix"],
    networking: ["src/firewall-rules.js"],
    kernel: ["src/pidfd-handler.rs"],
    build: ["Makefile"],
  };
  for (const [seat, paths] of Object.entries(triggerPaths)) {
    const result = selectRoster(candidate({ changed_paths: paths }));
    check(`the ${seat} path trigger selects its seat`, result.triggered_optional.includes(seat));
  }
  for (const path of [
    "src/routing-table.js",
    "src/mtu-policy.js",
    "src/mss-clamp.js",
  ]) {
    check(
      `the networking path trigger selects ${path}`,
      selectRoster(candidate({ changed_paths: [path] })).triggered_optional.includes("networking"),
    );
  }
  for (const path of [
    "rust-toolchain.toml",
    ".cargo/config.toml",
    "tests/layer1-jobs.json",
    "tests/test-rust.sh",
    "Makefile",
    "flake.nix",
    "packages/xtask/src/main.rs",
    "packages/xtask/src/delivery/panel.rs",
    "tests/static.sh",
    "tests/test-lint.sh",
  ]) {
    check(
      `the build seat path trigger selects ${path}`,
      selectRoster(candidate({ changed_paths: [path] })).triggered_optional.includes("build"),
    );
  }
  const signals = [
    "stateful",
    "agent",
    "nixos",
    "network",
    "routing",
    "mtu",
    "mss",
    "kernel",
    "build-contract",
  ];
  const signalSeats = [
    "reliability",
    "agentic",
    "nixos",
    "networking",
    "kernel",
    "build",
  ];
  const signalResult = selectRoster(candidate({ signals }));
  for (const seat of signalSeats) {
    check(`the ${seat} signal trigger selects its seat`, signalResult.triggered_optional.includes(seat));
  }
  check(
    "routing, mtu, and mss signals select networking",
    selectRoster(candidate({ signals: ["routing", "mtu", "mss"] }))
      .triggered_optional.includes("networking"),
  );
  const citation = selectRoster(
    candidate({
      changed_paths: ["docs/build-notes.md"],
      signals: ["citation", "build"],
    }),
  );
  check("citation-only prose does not select build", !citation.triggered_optional.includes("build"));

  const ambiguous = selectRoster(
    candidate({ changed_paths: ["src/panel.js"], ambiguous: true }),
  );
  check("ambiguous classification widens", ambiguous.ambiguity_widened);
  check("ambiguous classification uses the wider floor", ambiguous.roster.length === 10);
  const lifecycleSelection = selectLifecycleRoster({
    full_candidate: candidate(),
    fix_delta: { changed_paths: ["Makefile"] },
    previous_roster: code.roster,
  });
  check("lifecycle selection examines the full candidate and fix delta", lifecycleSelection.roster.includes("build"));

  const bmpPath = "\uE000.txt";
  const nonBmpPath = "\u{10000}.txt";
  const bmpSignal = "\uE000-signal";
  const nonBmpSignal = "\u{10000}-signal";
  const utf8Ordered = selectRoster(candidate({
    changed_paths: [nonBmpPath, bmpPath],
    signals: [nonBmpSignal, bmpSignal],
  }));
  check(
    "classification paths use UTF-8 byte ordering for BMP and non-BMP names",
    utf8Ordered.classification_inputs.changed_paths.join(",") ===
      [bmpPath, nonBmpPath].join(","),
  );
  check(
    "classification signals use UTF-8 byte ordering for BMP and non-BMP names",
    utf8Ordered.classification_inputs.signals.join(",") ===
      [bmpSignal, nonBmpSignal].join(","),
  );
  rejects(
    "classification inputs reject NUL-unrepresentable paths",
    () => selectRoster(candidate({ changed_paths: ["src/\u0000panel.js"] })),
    /control characters/,
  );
  rejects(
    "classification inputs reject C1 paths",
    () => selectRoster(candidate({ changed_paths: ["src/\u0080panel.js"] })),
    /control characters/,
  );
  rejects(
    "classification inputs reject C1 signals",
    () => selectRoster(candidate({ signals: ["build\u009f"] })),
    /control characters/,
  );

  const gitPathRoot = mkdtempSync(join(tmpdir(), "d2b-panel-git-paths-"));
  try {
    execFileSync("git", ["init", "--quiet"], { cwd: gitPathRoot });
    execFileSync("git", ["config", "user.name", "d2b test"], { cwd: gitPathRoot });
    execFileSync("git", ["config", "user.email", "d2b-test@example.invalid"], {
      cwd: gitPathRoot,
    });
    writeFileSync(join(gitPathRoot, "base.txt"), "base\n");
    execFileSync("git", ["add", "base.txt"], { cwd: gitPathRoot });
    execFileSync("git", ["commit", "--quiet", "-m", "base"], { cwd: gitPathRoot });
    const literalBackslashPath = "literal\\backslash.txt";
    writeFileSync(join(gitPathRoot, literalBackslashPath), "backslash\n");
    writeFileSync(join(gitPathRoot, bmpPath), "bmp\n");
    writeFileSync(join(gitPathRoot, nonBmpPath), "non-bmp\n");
    execFileSync("git", ["add", "."], { cwd: gitPathRoot });
    execFileSync("git", ["commit", "--quiet", "-m", "paths"], { cwd: gitPathRoot });
    const changedGitPaths = changedPathsFromGitRange("HEAD^..HEAD", gitPathRoot);
    const changedUnicodePaths = changedGitPaths.filter((path) =>
      [bmpPath, nonBmpPath].includes(path),
    );
    check(
      "git path derivation preserves literal backslashes",
      changedGitPaths.includes(literalBackslashPath),
    );
    check(
      "git path derivation sorts BMP and non-BMP names by UTF-8 bytes",
      changedUnicodePaths.join(",") === [bmpPath, nonBmpPath].join(","),
    );
    writeFileSync(join(gitPathRoot, "Makefile"), "all:\n\ttrue\n");
    execFileSync("git", ["add", "Makefile"], { cwd: gitPathRoot });
    execFileSync("git", ["commit", "--quiet", "-m", "build path"], {
      cwd: gitPathRoot,
    });
    execFileSync("git", ["mv", "Makefile", "notes.txt"], { cwd: gitPathRoot });
    execFileSync("git", ["commit", "--quiet", "-m", "rename build path"], {
      cwd: gitPathRoot,
    });
    const renamedPaths = changedPathsFromGitRange("HEAD^..HEAD", gitPathRoot);
    check(
      "git path derivation retains both sides of a rename",
      renamedPaths.includes("Makefile") && renamedPaths.includes("notes.txt"),
      renamedPaths.join(","),
    );
    const invalidUtf8Path = Buffer.concat([
      Buffer.from(`${gitPathRoot}/`),
      Buffer.from([0xc3, 0x28]),
    ]);
    writeFileSync(invalidUtf8Path, "invalid\n");
    execFileSync("git", ["add", "."], { cwd: gitPathRoot });
    execFileSync("git", ["commit", "--quiet", "-m", "invalid utf8"], {
      cwd: gitPathRoot,
    });
    rejects(
      "git path derivation rejects invalid UTF-8",
      () => changedPathsFromGitRange("HEAD^..HEAD", gitPathRoot),
      /invalid UTF-8/,
    );
    rejects(
      "git path derivation rejects NUL-unrepresentable ranges",
      () => changedPathsFromGitRange("HEAD\u0000..HEAD", gitPathRoot),
      /control character/,
    );
    rejects(
      "git path derivation rejects option-like ranges before invoking git",
      () => changedPathsFromGitRange("--stat", join(gitPathRoot, "missing-cwd")),
      /option-like/,
    );
  } finally {
    rmSync(gitPathRoot, { recursive: true, force: true });
  }
}

console.log("panel lifecycle: selection artifact");
const root = mkdtempSync(join(tmpdir(), "d2b-panel-lifecycle-"));
let priorSelection;
try {
  const initial = makeSelection(root);
  priorSelection = initial.selection;
  check(
    "selection is rendered at the candidate-bound lifecycle address",
    initial.path.endsWith(
      "/.scratch/panel/spec004w1/selections/candidate-1/" +
      `${"a".repeat(64)}.json`,
    ),
  );
  check("selection schema version is one", initial.selection.schema_version === 1);
  check("selection table version is two", initial.selection.selection_table_version === 2);
  check("selection is readable after rendering", readSelection(initial.path).candidate_id === "candidate-1");
  check(
    "identity-only current candidates remain valid staged addresses",
    validateCandidateAgainstSelection(initial.selection, {
      program: initial.selection.program,
      wave: initial.selection.wave,
      candidate_id: initial.selection.candidate_id,
      content_id: initial.selection.content_id,
      snapshot_sha256: initial.selection.snapshot_sha256,
    }) === true,
  );
  const repeated = makeSelection(root);
  check("identical selection regeneration is a compare", repeated.created === false);
  rejects(
    "conflicting selection regeneration is refused",
    () => makeSelection(root, { changed_paths: ["Makefile"] }),
    /conflicting generated bytes/,
  );

  const widened = createSelection(
    {
      ...candidate({
        snapshot_sha256: "b".repeat(64),
        changed_paths: ["Makefile"],
      }),
      lifecycle_id: "spec004w1",
      phase: "verification",
      previous_roster: initial.selection.roster,
    },
    { root },
  );
  check("build fix widening keeps every prior seat", initial.selection.roster.every((seat) => widened.selection.roster.includes(seat)));
  check("build fix widening adds build", widened.selection.roster.includes("build"));
  const nestedSelection = createSelection(
    {
      ...candidate({
        snapshot_sha256: "e".repeat(64),
        changed_paths: ["src/panel.js"],
        signals: ["rust"],
      }),
      lifecycle_id: "spec004w1",
      phase: "verification",
      full_candidate: candidate({
        snapshot_sha256: "e".repeat(64),
        changed_paths: ["src/panel.js"],
        signals: ["rust"],
      }),
      fix_delta: {
        changed_paths: ["docs/fix.md"],
        signals: [],
        candidate_class: "documentation",
        ambiguous: false,
      },
      previous_roster: initial.selection.roster,
    },
    { root },
  );
  check(
    "verification selection retains strict nested classification inputs",
    nestedSelection.selection.classification_inputs.full_candidate.candidate_class === "code" &&
      nestedSelection.selection.classification_inputs.fix_delta.candidate_class === "documentation",
  );
  const documentationFullConfigurationDelta = createSelection(
    {
      ...candidate({
        snapshot_sha256: "f".repeat(64),
        changed_paths: ["docs/full-review.md", "nixos/panel.nix"],
      }),
      lifecycle_id: "spec004w1",
      phase: "verification",
      full_candidate: {
        ...candidate({
          snapshot_sha256: "f".repeat(64),
          changed_paths: ["docs/full-review.md"],
        }),
        changed_paths: ["docs/full-review.md"],
        signals: [],
        candidate_class: "documentation",
        ambiguous: false,
      },
      fix_delta: {
        changed_paths: ["nixos/panel.nix"],
        signals: [],
        candidate_class: "configuration",
        ambiguous: false,
      },
      previous_roster: initial.selection.roster,
    },
    { root },
  );
  check(
    "verification class precedence keeps configuration over documentation",
    documentationFullConfigurationDelta.selection.candidate_class === "configuration" &&
      documentationFullConfigurationDelta.selection.classification_inputs.changed_paths.join(",") ===
        "docs/full-review.md,nixos/panel.nix",
  );
  rejects(
    "verification selections require both nested classifications",
    () => {
      const incomplete = { ...nestedSelection.selection };
      const classification = {
        ...incomplete.classification_inputs,
      };
      delete classification.full_candidate;
      incomplete.classification_inputs = classification;
      validateSelection(incomplete);
    },
    /both.*full_candidate.*fix_delta/,
  );
  rejects(
    "classification signals must already be lowercase and trimmed",
    () => validateSelection({
      ...nestedSelection.selection,
      classification_inputs: {
        ...nestedSelection.selection.classification_inputs,
        full_candidate: {
          ...nestedSelection.selection.classification_inputs.full_candidate,
          signals: ["Rust"],
        },
      },
    }),
    /canonical normalized signals/,
  );
  const literalBackslashSelection = createSelection(
    {
      ...candidate({
        snapshot_sha256: "6".repeat(64),
        changed_paths: ["src\\panel.js"],
      }),
      lifecycle_id: "spec004w1",
      phase: "discovery",
    },
    { root },
  );
  check(
    "classification paths preserve literal backslashes",
    literalBackslashSelection.selection.classification_inputs.changed_paths[0] ===
      "src\\panel.js" &&
      readSelection(literalBackslashSelection.path).classification_inputs.changed_paths[0] ===
        "src\\panel.js",
  );
  rejects(
    "actual non-documentation paths cannot be narrowed to documentation",
    () => selectRoster(candidate({
      changed_paths: ["src/panel.js"],
      candidate_class: "documentation",
    })),
    /cannot narrow actual non-documentation paths/,
  );
  rejects(
    "nested documentation classifications cannot narrow source paths",
    () => validateSelection({
      ...nestedSelection.selection,
      classification_inputs: {
        ...nestedSelection.selection.classification_inputs,
        fix_delta: {
          ...nestedSelection.selection.classification_inputs.fix_delta,
          changed_paths: ["src/fix.js"],
        },
      },
    }),
    /candidate_class documentation cannot narrow actual non-documentation paths/,
  );
  const recursiveRoot = mkdtempSync(join(tmpdir(), "d2b-panel-recursive-docs-"));
  try {
    const recursiveDocumentation = createSelection(
      {
        ...candidate({
          snapshot_sha256: "7".repeat(64),
          changed_paths: ["docs/nested/review.md"],
        }),
        lifecycle_id: "spec004w1",
        phase: "discovery",
      },
      { root: recursiveRoot },
    );
    check(
      "nested documentation paths retain documentation classification",
      recursiveDocumentation.selection.candidate_class === "documentation",
    );
  } finally {
    rmSync(recursiveRoot, { recursive: true, force: true });
  }
  rejects(
    "nested classification unknown fields are refused",
    () => validateSelection({
      ...nestedSelection.selection,
      classification_inputs: {
        ...nestedSelection.selection.classification_inputs,
        full_candidate: {
          ...nestedSelection.selection.classification_inputs.full_candidate,
          unexpected: true,
        },
      },
    }),
    /unknown field/,
  );
  rejects(
    "nested classification path unions are refused",
    () => validateSelection({
      ...nestedSelection.selection,
      classification_inputs: {
        ...nestedSelection.selection.classification_inputs,
        changed_paths: ["src/panel.js"],
      },
    }),
    /union.*paths/,
  );
  rejects(
    "nested classification signal unions are refused",
    () => validateSelection({
      ...nestedSelection.selection,
      classification_inputs: {
        ...nestedSelection.selection.classification_inputs,
        signals: [],
      },
    }),
    /union.*signals/,
  );
  rejects(
    "nested classification class and ambiguity mismatches are refused",
    () => validateSelection({
      ...nestedSelection.selection,
      classification_inputs: {
        ...nestedSelection.selection.classification_inputs,
        fix_delta: {
          ...nestedSelection.selection.classification_inputs.fix_delta,
          candidate_class: "ambiguous",
        },
      },
    }),
    /candidate_class and ambiguous disagree/,
  );
  rejects(
    "roster narrowing is refused",
    () => validateMonotonicRoster(
      initial.selection.roster,
      initial.selection.roster.slice(0, -1),
      readSelectionTable(),
    ),
    /roster narrowing|mandatory/,
  );

  console.log("panel lifecycle: comprehensive discovery and ledger");
  const request = createDiscoveryRequest({
    selection: initial.selection,
    candidate: candidate(),
    context: { full_candidate: "staged" },
    validation_evidence: ["node tests"],
  });
  check("discovery request is comprehensive", request.comprehensive === true && request.full_candidate === true);
  check("discovery request states the one comprehensive instruction", /comprehensive/.test(request.instruction) && /later rounds/.test(request.instruction));
  const actualDiscoveryVerdict = {
    engineer: "software",
    signoff: false,
    summary: "A source mapping issue was found.",
    recommendations: [{
      severity: "high",
      where: "scripts/panel.js:1",
      what: "A source can disappear.",
      why: "The ledger would be incomplete.",
      fix: "Validate source coverage.",
    }],
  };
  const adaptedDiscovery = adaptDiscoveryVerdict(actualDiscoveryVerdict);
  check(
    "actual verdict JSON adapts to a complete discovery result",
    adaptedDiscovery.complete === true &&
      adaptedDiscovery.seat === "software" &&
      adaptedDiscovery.findings[0].severity === "MAJOR",
  );
  rejects(
    "an inconsistent actual discovery signoff is refused",
    () => adaptDiscoveryVerdict({ ...actualDiscoveryVerdict, signoff: true }),
    /signoff/,
  );
  rejects(
    "a current discovery recommendation must be an object",
    () => adaptDiscoveryVerdict({
      ...actualDiscoveryVerdict,
      recommendations: ["Validate source coverage."],
    }),
    /recommendations\[0\].*object/,
  );
  for (const field of ["severity", "where", "what", "why", "fix"]) {
    const recommendation = { ...actualDiscoveryVerdict.recommendations[0] };
    delete recommendation[field];
    rejects(
      `a current discovery recommendation requires explicit ${field}`,
      () => adaptDiscoveryVerdict({
        ...actualDiscoveryVerdict,
        recommendations: [recommendation],
      }),
      new RegExp(field),
    );
  }
  rejects(
    "a current recommendation severity must use the exact reviewer vocabulary",
    () => adaptDiscoveryVerdict({
      ...actualDiscoveryVerdict,
      recommendations: [{
        ...actualDiscoveryVerdict.recommendations[0],
        severity: "MAJOR",
      }],
    }),
    /severity.*exactly/,
  );
  rejects(
    "a current recommendation refuses undeclared fields",
    () => adaptDiscoveryVerdict({
      ...actualDiscoveryVerdict,
      recommendations: [{
        ...actualDiscoveryVerdict.recommendations[0],
        impact: "Legacy alias must not be inferred.",
      }],
    }),
    /fields.*impact.*expected exactly/,
  );
  const duplicateDiscoveryVerdicts = initial.selection.roster.map((seat) => ({
    engineer: seat,
    signoff: true,
    summary: "No findings.",
    recommendations: [],
  }));
  duplicateDiscoveryVerdicts.push({ ...duplicateDiscoveryVerdicts[0] });
  rejects(
    "duplicate current discovery seats are refused before keyed conversion",
    () => validateDiscoveryResults(initial.selection, duplicateDiscoveryVerdicts),
    /duplicate discovery verdict.*seat/,
  );

  const zeroResults = completeResults(initial.selection.roster);
  const zeroSources = validateDiscoveryResults(initial.selection, zeroResults);
  check("explicit zero-finding results are accepted", zeroSources.length === 0);
  const withFinding = completeResults(initial.selection.roster, "software");
  const testResult = withFinding.test;
  testResult.findings = [{
    source_id: "test:1",
    source_ordinal: 1,
    raw_text: "The generated request can lose a source.",
    attribution: "test",
    severity: "MAJOR",
    impact: "A source could disappear from the shared ledger.",
    recommendation: "Validate the source mapping before generation.",
  }];
  rejects(
    "a missing selected-seat discovery result is refused",
    () => {
      const missing = { ...withFinding };
      delete missing[initial.selection.roster.at(-1)];
      validateDiscoveryResults(initial.selection, missing);
    },
    /missing complete discovery result/,
  );
  rejects(
    "an incomplete selected-seat discovery result is refused",
    () => validateDiscoveryResults(initial.selection, {
      ...withFinding,
      software: { ...withFinding.software, complete: false },
    }),
    /complete: true/,
  );
  const groups = [
    {
      source_finding_ids: ["software:1", "test:1"],
      description: "One shared source-mapping defect.",
      severity: "MAJOR",
      impact: "The ledger could lose attribution.",
      recommendation: "Validate the exact source mapping.",
    },
  ];
  const ledger = mergeDiscoveryLedger({
    selection: initial.selection,
    results: withFinding,
    groups,
  });
  check("deduplication creates one stable R identifier", ledger.issues[0].id === "R1");
  check("deduplication preserves both source attributions", ledger.issues[0].source_finding_ids.join(",") === "software:1,test:1");
  const ledgerAgain = mergeDiscoveryLedger({
    selection: initial.selection,
    results: withFinding,
    groups,
  });
  check("identical ledger inputs are byte-stable", stableStringify(ledger) === stableStringify(ledgerAgain));
  rejects(
    "an unmapped source finding is refused",
    () => mergeDiscoveryLedger({
      selection: initial.selection,
      results: withFinding,
      groups: [{ source_finding_ids: ["software:1"] }],
    }),
    /mapping is incomplete/,
  );
  rejects(
    "a source mapped into two groups is refused",
    () => mergeDiscoveryLedger({
      selection: initial.selection,
      results: withFinding,
      groups: [
        { source_finding_ids: ["software:1", "test:1"] },
        { source_finding_ids: ["software:1"] },
      ],
    }),
    /more than one/,
  );
  rejects(
    "a ledger group cannot downgrade source severity",
    () => mergeDiscoveryLedger({
      selection: initial.selection,
      results: withFinding,
      groups: [{
        source_finding_ids: ["software:1", "test:1"],
        severity: "MINOR",
      }],
    }),
    /maximum source severity/,
  );
  const ledgerPath = join(root, "ledger.json");
  writeCreateOrCompare(ledgerPath, ledger);
  rejects(
    "conflicting ledger bytes are refused",
    () => writeCreateOrCompare(ledgerPath, { ...ledger, complete: false }),
    /conflicting generated bytes/,
  );

  console.log("panel lifecycle: responses and strict acceptance");
  const responseInput = makeLedger();
  const fixed = {
    issue_id: "R1",
    disposition: "Fixed",
    changed_surface: ["src/panel.js"],
    justification: "The source mapping is now validated.",
    evidence: "targeted Node test",
  };
  const factual = {
    issue_id: "R2",
    disposition: "Invalid",
    justification: "The report describes behavior that is not present.",
    verified_factual_status: "Verified against the full candidate.",
    evidence: "source inspection",
  };
  const minor = {
    issue_id: "R3",
    disposition: "Deferred",
    justification: "This non-blocking cleanup is recorded for later.",
  };
  const nit = {
    issue_id: "R4",
    disposition: "Withdrawn",
    justification: "The wording is already correct.",
    verified_factual_status: "Verified against the candidate.",
    evidence: "source inspection",
  };
  const responses = [fixed, factual, minor, nit];
  check("all supported response dispositions validate", validateResponses(responseInput, responses).length === 4);
  check(
    "response template covers every ledger issue",
    createResponseTemplate(responseInput).responses.length === responseInput.issues.length,
  );
  check("fixed blocker is approvable", evaluateApproval({
    selection: initial.selection,
    ledger: responseInput,
    responses,
    verification_results: allVerificationResults(initial.selection.roster),
  }).approved === true);
  const approvalArtifact = createApprovalArtifact({
    selection: initial.selection,
    ledger: responseInput,
    responses,
    ledger_bytes: stableStringify(responseInput),
    responses_bytes: stableStringify(responses),
    verification_results: adaptedVerificationArtifact(
      initial.selection,
      responseInput,
      allVerificationResults(initial.selection.roster, {}, responseInput),
    ),
    verification_results_bytes: stableStringify(
      adaptedVerificationArtifact(
        initial.selection,
        responseInput,
        allVerificationResults(initial.selection.roster, {}, responseInput),
      ),
    ),
  });
  check(
    "approval is exposed as a directly consumable artifact",
    approvalArtifact.artifact_kind === "d2b-panel/approval" &&
      approvalArtifact.approved === true &&
      approvalArtifact.selection_sha256.length === 64,
  );
  for (const disposition of ["Invalid", "Withdrawn"]) {
    const factualBlocker = {
      issue_id: "R1",
      disposition,
      justification: `The BLOCKER is factually ${disposition.toLowerCase()}.`,
      verified_factual_status: "Verified against the full candidate.",
      evidence: "source inspection",
    };
    check(
      `verified ${disposition} BLOCKER is approvable`,
      evaluateApproval({
        selection: initial.selection,
        ledger: responseInput,
        responses: [factualBlocker, factual, minor, nit],
        verification_results: allVerificationResults(initial.selection.roster),
      }).approved === true,
    );
  }
  rejects(
    "a missing ledger response is refused",
    () => validateResponses(responseInput, responses.slice(0, -1)),
    /missing implementation responses/,
  );
  rejects(
    "fixed without evidence is refused",
    () => validateResponses(responseInput, [{ ...fixed, evidence: "" }, factual, minor, nit]),
    /requires non-blank evidence/,
  );
  rejects(
    "invalid without factual verification is refused",
    () => validateResponses(responseInput, [{ ...fixed }, { ...factual, verified_factual_status: "" }, minor, nit]),
    /requires verified_factual_status/,
  );
  rejects(
    "deferred blocker cannot be accepted",
    () => {
      const result = evaluateApproval({
        selection: initial.selection,
        ledger: responseInput,
        responses: [
          { ...fixed, issue_id: "R1", disposition: "Deferred", changed_surface: undefined },
          factual,
          minor,
          nit,
        ],
        verification_results: allVerificationResults(initial.selection.roster),
      });
      if (result.blocking_issues.includes("R1")) {
        throw new Error("Deferred BLOCKER approved");
      }
    },
    /Deferred BLOCKER approved/,
  );

  const majorLedger = {
    ...responseInput,
    issues: [{ ...responseInput.issues[1], id: "R1" }],
    sources: [responseInput.sources[1]],
  };
  const acceptedMajor = {
    issue_id: "R1",
    disposition: "Deferred",
    justification: "The merge owner accepts the documented residual risk.",
    acceptance: {
      accepter: "merge-owner",
      capacity: "merge owner",
      justification: "The risk is bounded and tracked.",
    },
  };
  check("accepted Deferred MAJOR validates", validateResponses(majorLedger, [acceptedMajor]).length === 1);
  check("accepted Deferred MAJOR approves", evaluateApproval({
    selection: initial.selection,
    ledger: majorLedger,
    responses: [acceptedMajor],
    verification_results: allVerificationResults(initial.selection.roster, {}, majorLedger),
  }).approved === true);
  const acceptedRejectedMajor = {
    ...acceptedMajor,
    disposition: "Intentionally rejected",
  };
  check(
    "accepted Intentionally rejected MAJOR approves",
    evaluateApproval({
      selection: initial.selection,
      ledger: majorLedger,
      responses: [acceptedRejectedMajor],
      verification_results: allVerificationResults(initial.selection.roster, {}, majorLedger),
    }).approved === true,
  );
  for (const disposition of ["Invalid", "Withdrawn"]) {
    const factualMajor = {
      issue_id: "R1",
      disposition,
      justification: `The MAJOR is factually ${disposition.toLowerCase()}.`,
      verified_factual_status: "Verified against the full candidate.",
      evidence: "source inspection",
    };
    check(
      `verified ${disposition} MAJOR needs no acceptance`,
      evaluateApproval({
        selection: initial.selection,
        ledger: majorLedger,
        responses: [factualMajor],
        verification_results: allVerificationResults(initial.selection.roster, {}, majorLedger),
      }).approved === true,
    );
  }
  const acceptanceMutations = [
    undefined,
    null,
    [],
    "accepted",
    { capacity: "merge owner", justification: "x" },
    { accepter: "x", capacity: "merge owner" },
    { accepter: "x", capacity: "merge owner", justification: "x", extra: "no" },
    { accepter: 1, capacity: "merge owner", justification: "x" },
    { accepter: "x", capacity: 1, justification: "x" },
    { accepter: "x", capacity: "merge owner", justification: 1 },
    { accepter: " ", capacity: "merge owner", justification: "x" },
    { accepter: "x", capacity: "merge owner", justification: " " },
    { accepter: "x", capacity: "", justification: "x" },
    { accepter: "x", capacity: " ", justification: "x" },
    { accepter: "x", capacity: "repository owner", justification: "x" },
  ];
  for (const [index, acceptance] of acceptanceMutations.entries()) {
    rejects(
      `malformed acceptance ${index + 1} is refused`,
      () => validateResponses(majorLedger, [{ ...acceptedMajor, acceptance }]),
      /acceptance|capacity/,
    );
  }
  rejects(
    "Intentionally rejected without acceptance is refused at approval",
    () => {
      const result = evaluateApproval({
        selection: initial.selection,
        ledger: majorLedger,
        responses: [{ ...acceptedMajor, disposition: "Intentionally rejected", acceptance: undefined }],
        verification_results: allVerificationResults(initial.selection.roster, {}, majorLedger),
      });
      if (result.approved) throw new Error("unaccepted MAJOR approved");
    },
    /requires acceptance/,
  );

  console.log("panel lifecycle: self-verification, scope, late findings, and metrics");
  const selfVerification = {
    tests: ["node scripts/copilot/test-panel-lifecycle.mjs"],
    lint: "shell syntax and Node checks passed",
    formatting: "not applicable",
    static_analysis: "check-bindings planned for integration",
    build: "not applicable",
    uncovered_areas: ["delivery xtask is another slice"],
    self_review: "inspected the complete changed surface",
  };
  check("self-verification requires and accepts every field", validateSelfVerification(selfVerification).build === "not applicable");
  rejects(
    "self-verification with a missing field is refused",
    () => validateSelfVerification({ ...selfVerification, build: undefined }),
    /build/,
  );
  check(
    "fix scope accepts declared paths",
    validateFixScope({ latest_delta_paths: ["src/panel.js"], allowed_paths: ["src/panel.js"] }).latest_delta_paths.length === 1,
  );
  rejects(
    "unrelated fix scope is refused",
    () => validateFixScope({ latest_delta_paths: ["src/unrelated.js"], allowed_paths: ["src/panel.js"] }),
    /unrelated paths|new lifecycle/,
  );
  rejects(
    "pre-existing late NIT is refused",
    () => lateFindingAdmission({ severity: "NIT", category: "style", previously_missed: true }),
    /not admissible/,
  );
  check(
    "introduced late NIT is admitted as a non-discovery regression",
    lateFindingAdmission({ severity: "NIT", introduced_regression: true }).late === true,
  );
  check(
    "previously missed late MAJOR is admitted",
    lateFindingAdmission({ severity: "MAJOR", previously_missed: true }).late === true,
  );
  const appended = appendLateFindings(responseInput, [{
    severity: "MAJOR",
    previously_missed: true,
    seat: "software",
    raw_text: "A late unsafe issue.",
    impact: "Approval would be unsafe.",
    recommendation: "Fix the issue.",
  }]);
  check("late issue receives the next stable R identifier", appended.issues.at(-1).id === "R5");
  rejects(
    "re-admitting the same late source is refused",
    () => appendLateFindings(appended, [{
      severity: "MAJOR",
      previously_missed: true,
      seat: "software",
      raw_text: "A late unsafe issue.",
      impact: "Approval would be unsafe.",
      recommendation: "Fix the issue.",
    }]),
    /already exists/,
  );
  const metrics = calculateMetrics({
    ledger: appended,
    responses: appended.issues.map((issue) => ({
      issue_id: issue.id,
      disposition: "Fixed",
      changed_surface: ["src/panel.js"],
      justification: "The issue was fixed.",
      evidence: "focused test",
    })),
    review_iterations: 2,
    implementation_iterations: 2,
  });
  check("metrics count late blockers and majors", metrics.late_major_count === 1 && metrics.late_unique_findings === 1);
  check("metrics calculate fixed issues per iteration", metrics.average_fixed_issues_per_implementation_iteration === 2.5);
  check("zero implementation iterations produce 0.0", calculateMetrics({ ledger: responseInput, implementation_iterations: 0 }).average_fixed_issues_per_implementation_iteration === 0.0);

  console.log("panel lifecycle: scoped verification");
  const verificationSelection = createSelection(
    {
      ...candidate({ snapshot_sha256: "c".repeat(64), changed_paths: ["src/panel.js"] }),
      lifecycle_id: "spec004w1",
      phase: "verification",
      previous_roster: initial.selection.roster,
      fix_delta: { changed_paths: ["src/panel.js"] },
    },
    { root },
  );
  const priorVerdicts = Object.fromEntries(
    initial.selection.roster.map((seat) => [
      seat,
      {
        engineer: seat,
        signoff: true,
        summary: "Prior verification passed.",
        recommendations: [],
      },
    ]),
  );
  const verificationInput = prepareVerification({
    selection: verificationSelection.selection,
    ledger: { ...responseInput, snapshot_sha256: "c".repeat(64) },
    responses,
    self_verification: selfVerification,
    current_candidate: candidate({
      snapshot_sha256: "c".repeat(64),
      content_id: "content-1",
    }),
    prior_selection: initial.selection,
    prior_verdicts: priorVerdicts,
    latest_delta_paths: ["src/panel.js"],
  });
  check("verification receives the complete ledger", verificationInput.requests[0].ledger.issues.length === 4);
  check("verification keeps discovery closed", verificationInput.requests[0].comprehensive_discovery_already_complete === true);
  check(
    "verification requests carry exact incumbent prior verdicts only",
    verificationInput.requests.every((request) =>
      request.previous_status === priorVerdicts[request.seat] &&
      request.actual_delta.context === undefined &&
      request.full_context === undefined,
    ),
  );
  check(
    "strict verification request validation binds every staged input",
    validateVerificationRequest(verificationInput.requests[0], {
      selection: verificationSelection.selection,
      ledger: { ...responseInput, snapshot_sha256: "c".repeat(64) },
      responses,
      self_verification: selfVerification,
      current_candidate: candidate({
        snapshot_sha256: "c".repeat(64),
        content_id: "content-1",
      }),
      prior_selection: initial.selection,
      previous_status: priorVerdicts[verificationInput.requests[0].seat],
    }).seat === verificationInput.requests[0].seat,
  );
  const verified = validateVerificationResults(
    verificationSelection.selection,
    allVerificationResults(verificationSelection.selection.roster),
    { ledger: { ...responseInput, snapshot_sha256: "c".repeat(64) } },
  );
  check("every selected seat supplies complete verification", verified.length === verificationSelection.selection.roster.length);
  const verificationStatuses = Object.fromEntries(
    responseInput.issues.map((issue) => [issue.id, "resolved"]),
  );
  const actualVerificationVerdict = adaptVerificationVerdict({
    engineer: "software",
    signoff: true,
    summary: "All ledger issues were verified.",
    issue_statuses: verificationStatuses,
    recommendations: [],
  }, { issue_ids: responseInput.issues.map((issue) => issue.id) });
  check(
    "actual verdict JSON adapts to explicit verification status",
    actualVerificationVerdict.verified_issue_statuses.R1 === "resolved" &&
      actualVerificationVerdict.signoff === true,
  );
  rejects(
    "a current verification recommendation must use the strict object shape",
    () => adaptVerificationVerdict({
      ...actualVerificationVerdict,
      signoff: false,
      recommendations: ["Fix the unresolved issue."],
    }, { issue_ids: responseInput.issues.map((issue) => issue.id) }),
    /recommendations\[0\].*object/,
  );
  const malformedVerificationResults = allVerificationResults(
    verificationSelection.selection.roster,
  );
  malformedVerificationResults.software = {
    ...malformedVerificationResults.software,
    signoff: false,
    recommendations: [{
      severity: "high",
      what: "A direct verification result omitted its location.",
      why: "The recommendation cannot be actioned precisely.",
      fix: "Declare the exact location.",
    }],
  };
  rejects(
    "direct current verification results cannot bypass recommendation shape",
    () => validateVerificationResults(
      verificationSelection.selection,
      malformedVerificationResults,
      { ledger: { ...responseInput, snapshot_sha256: "c".repeat(64) } },
    ),
    /recommendations\[0\].*where/,
  );
  const duplicateVerificationVerdicts =
    verificationSelection.selection.roster.map((seat) => ({
      engineer: seat,
      signoff: true,
      summary: "All ledger issues were verified.",
      issue_statuses: verificationStatuses,
      recommendations: [],
    }));
  duplicateVerificationVerdicts.push({ ...duplicateVerificationVerdicts[0] });
  rejects(
    "duplicate current verification seats are refused before keyed conversion",
    () => validateVerificationResults(
      verificationSelection.selection,
      duplicateVerificationVerdicts,
      { ledger: { ...responseInput, snapshot_sha256: "c".repeat(64) } },
    ),
    /duplicate verification verdict.*seat/,
  );
  const noOpCandidate = candidate({
    snapshot_sha256: "d".repeat(64),
    content_id: "content-1",
  });
  const noOpVerificationSelection = createSelection(
    {
      ...noOpCandidate,
      lifecycle_id: "spec004w1",
      phase: "verification",
      previous_roster: initial.selection.roster,
    },
    { root },
  );
  const noOpLedger = {
    ...responseInput,
    snapshot_sha256: noOpCandidate.snapshot_sha256,
  };
  const noOpPreparationInput = {
    selection: noOpVerificationSelection.selection,
    ledger: noOpLedger,
    responses,
    self_verification: selfVerification,
    current_candidate: noOpCandidate,
    prior_selection: initial.selection,
    prior_verdicts: priorVerdicts,
    latest_delta_paths: [],
  };
  const noOpVerification = prepareVerification(noOpPreparationInput);
  check(
    "an exact empty no-op verification delta is preserved",
    noOpVerification.scope.latest_delta_paths.length === 0 &&
      noOpVerification.requests.every((request) =>
        request.latest_delta_paths.length === 0 &&
        request.actual_delta.paths.length === 0 &&
        request.fix_delta.changed_paths.length === 0
      ),
  );
  check(
    "strict request validation accepts the declared empty no-op delta",
    validateVerificationRequest(noOpVerification.requests[0], {
      selection: noOpVerificationSelection.selection,
      ledger: noOpLedger,
      responses,
      self_verification: selfVerification,
      current_candidate: noOpCandidate,
      prior_selection: initial.selection,
      previous_status: priorVerdicts[noOpVerification.requests[0].seat],
    }).seat === noOpVerification.requests[0].seat,
  );
  rejects(
    "verification preparation rejects an undeclared delta for a no-op selection",
    () => prepareVerification({
      ...noOpPreparationInput,
      latest_delta_paths: ["src/panel.js"],
    }),
    /must equal.*declared.*fix delta/,
  );
  rejects(
    "verification request validation rejects an undeclared no-op delta",
    () => validateVerificationRequest({
      ...noOpVerification.requests[0],
      latest_delta_paths: ["src/panel.js"],
      actual_delta: { paths: ["src/panel.js"] },
      fix_delta: { changed_paths: ["src/panel.js"] },
    }, {
      selection: noOpVerificationSelection.selection,
      ledger: noOpLedger,
      responses,
      self_verification: selfVerification,
      current_candidate: noOpCandidate,
      prior_selection: initial.selection,
      previous_status: priorVerdicts[noOpVerification.requests[0].seat],
    }),
    /latest_delta_paths disagree with selection/,
  );
  const inconsistentStatusResults = allVerificationResults(
    verificationSelection.selection.roster,
    {},
    { ...responseInput, snapshot_sha256: "c".repeat(64) },
  );
  inconsistentStatusResults.software.verified_issue_statuses.R1 = "accepted";
  const inconsistentStatusApproval = evaluateApproval({
    selection: verificationSelection.selection,
    ledger: { ...responseInput, snapshot_sha256: "c".repeat(64) },
    responses,
    verification_results: inconsistentStatusResults,
  });
  check(
    "only resolved or verified issue statuses can pass approval",
    inconsistentStatusApproval.approved === false &&
      inconsistentStatusApproval.status_blocks.some((item) => item.issue_id === "R1"),
  );
  rejects(
    "verification refuses a missing per-issue status",
    () => validateVerificationResults(
      verificationSelection.selection,
      {
        software: {
          ...allVerificationResults(verificationSelection.selection.roster).software,
          verified_issue_statuses: { R1: "resolved" },
        },
      },
      { ledger: { ...responseInput, snapshot_sha256: "c".repeat(64) } },
    ),
    /cover each issue|missing/,
  );
  const lateApproval = evaluateApproval({
    selection: verificationSelection.selection,
    ledger: { ...responseInput, snapshot_sha256: "c".repeat(64) },
    responses,
    verification_results: Object.fromEntries(
      verificationSelection.selection.roster.map((seat) => [
        seat,
        {
          ...allVerificationResults(verificationSelection.selection.roster).software,
          seat,
          verified_issue_statuses: verificationStatuses,
          late_findings: seat === "software"
            ? [{
                severity: "MAJOR",
                introduced_regression: true,
                raw_text: "A late blocking regression.",
                impact: "The current candidate is unsafe.",
                recommendation: "Fix the regression.",
              }]
            : [],
        },
      ]),
    ),
  });
  check(
    "admitted late MAJOR is appended and blocks approval",
    lateApproval.late_blocking_issues.length === 1 &&
      lateApproval.approved === false &&
      lateApproval.ledger.issues.at(-1).late === true,
  );
  const verificationDir = join(root, "verification");
  const writtenVerification = writeVerificationArtifacts(verificationDir, {
    selection: verificationSelection.selection,
    ledger: { ...responseInput, snapshot_sha256: "c".repeat(64) },
    responses,
    self_verification: selfVerification,
    current_candidate: candidate({
      snapshot_sha256: "c".repeat(64),
      content_id: "content-1",
    }),
    prior_selection: initial.selection,
    prior_verdicts: priorVerdicts,
    latest_delta_paths: ["src/panel.js"],
  });
  check(
    "verification generation writes one request per selected seat",
    writtenVerification.written.length === verificationSelection.selection.roster.length,
  );
  rmSync(join(verificationDir, "software.json"));
  rejects(
    "verification artifact family refuses a partial existing directory",
    () => writeVerificationArtifacts(verificationDir, {
      selection: verificationSelection.selection,
      ledger: { ...responseInput, snapshot_sha256: "c".repeat(64) },
      responses,
      self_verification: selfVerification,
      current_candidate: candidate({
        snapshot_sha256: "c".repeat(64),
        content_id: "content-1",
      }),
      prior_selection: initial.selection,
      prior_verdicts: priorVerdicts,
      latest_delta_paths: ["src/panel.js"],
    }),
    /incomplete or has extra entries/,
  );
  rejects(
    "a missing verification seat is refused",
    () => validateVerificationResults(
      verificationSelection.selection,
      Object.fromEntries(
        Object.entries(allVerificationResults(verificationSelection.selection.roster)).slice(0, -1),
      ),
      { ledger: { ...responseInput, snapshot_sha256: "c".repeat(64) } },
    ),
    /missing verification result/,
  );
  const raceDirectory = join(root, "race-family");
  const raceResults = await concurrentDirectoryPublish(raceDirectory, LIFECYCLE_CLI);
  const raceBytes = readFileSync(join(raceDirectory, "seat.json"), "utf8");
  check(
    "concurrent directory publishers have exactly one atomic winner",
    raceResults.filter((result) =>
      result.status === 0 && /"created":true/.test(result.stdout),
    ).length === 1 &&
      ["first\n", "second\n"].includes(raceBytes) &&
      !existsSync(`${raceDirectory}.claim`) &&
      raceResults.every((result) => result.status === 0 || result.status === 1),
    raceResults.map((result) => `${result.stdout}${result.stderr}`).join(" "),
  );
  const existingComparison = await compareExistingDirectory(
    raceDirectory,
    LIFECYCLE_CLI,
    raceBytes,
  );
  check(
    "an existing complete directory is compared without replacement",
    existingComparison.status === 0 &&
      /"created":false/.test(existingComparison.stdout),
    `${existingComparison.stdout}${existingComparison.stderr}`,
  );
  const observedDirectory = join(root, "observed-family");
  const observation = await observeDirectoryPublish(observedDirectory, LIFECYCLE_CLI);
  const observedEntries = readdirSync(observedDirectory).sort();
  check(
    "directory observers see either no destination or a complete family",
    observation.status === 0 &&
      observation.violation === "" &&
      observedEntries.join(",") === "first.json,second.json" &&
      readdirSync(observedDirectory).every((name) =>
        readFileSync(join(observedDirectory, name)).length === 8 * 1024 * 1024),
    observation.violation || observation.stderr,
  );
  const faultDirectory = join(root, "unavailable-mv-family");
  const noMvPath = join(root, "no-mv-bin");
  mkdirSync(noMvPath);
  const fault = await unavailableDirectoryPublish(
    faultDirectory,
    LIFECYCLE_CLI,
    noMvPath,
  );
  const faultSiblings = readdirSync(dirname(faultDirectory))
    .filter((name) => name.startsWith(`.${basename(faultDirectory)}.stage-`));
  check(
    "an unavailable atomic primitive fails clearly and cleans its staging state",
    fault.status === 1 &&
      !existsSync(faultDirectory) &&
      !existsSync(`${faultDirectory}.claim`) &&
      faultSiblings.length === 0 &&
      /requires GNU mv.*no-clobber.*no-target-directory/.test(fault.stderr),
    fault.stderr,
  );
  const staleDirectory = join(root, "stale-family");
  mkdirSync(`${staleDirectory}.claim`);
  rejects(
    "a stale sibling claim names the required cleanup",
    () => writeDirectoryCreateOrCompare(
      staleDirectory,
      [{ name: "seat.json", bytes: "stale\n" }],
    ),
    /stale.*rm -rf --/,
  );
  rmSync(`${staleDirectory}.claim`, { recursive: true, force: true });
  rejects(
    "verification requires an explicit current candidate",
    () => prepareVerification({
      selection: verificationSelection.selection,
      ledger: { ...responseInput, snapshot_sha256: "c".repeat(64) },
      responses,
      self_verification: selfVerification,
      prior_selection: initial.selection,
      latest_delta_paths: ["src/panel.js"],
    }),
    /explicit current candidate/,
  );
  rejects(
    "verification requires an explicit prior selection",
    () => prepareVerification({
      selection: verificationSelection.selection,
      ledger: { ...responseInput, snapshot_sha256: "c".repeat(64) },
      responses,
      self_verification: selfVerification,
      current_candidate: candidate({ snapshot_sha256: "c".repeat(64) }),
      latest_delta_paths: ["src/panel.js"],
    }),
    /explicit prior selection/,
  );
  rejects(
    "verification actual delta must equal the declared non-empty delta",
    () => prepareVerification({
      selection: verificationSelection.selection,
      ledger: { ...responseInput, snapshot_sha256: "c".repeat(64) },
      responses,
      self_verification: selfVerification,
      current_candidate: candidate({ snapshot_sha256: "c".repeat(64) }),
      prior_selection: initial.selection,
      prior_verdicts: priorVerdicts,
      latest_delta_paths: [],
    }),
    /must equal.*declared.*fix delta/,
  );
  rejects(
    "verification requires prior verdicts when a prior selection is supplied",
    () => prepareVerification({
      selection: verificationSelection.selection,
      ledger: { ...responseInput, snapshot_sha256: "c".repeat(64) },
      responses,
      self_verification: selfVerification,
      current_candidate: candidate({ snapshot_sha256: "c".repeat(64) }),
      prior_selection: initial.selection,
      latest_delta_paths: ["src/panel.js"],
    }),
    /explicit prior verdicts/,
  );

  console.log("panel lifecycle: public CLI integration");
  const cliRoot = mkdtempSync(join(tmpdir(), "d2b-panel-cli-"));
  try {
    const cliStageScript = join(
      fileURLToPath(new URL(".", import.meta.url)),
      "..",
      "..",
      ".github",
      "skills",
      "d2b-panel-round",
      "scripts",
      "stage-diffs.sh",
    );
    const cliLifecycleDir = join(cliRoot, ".github", "skills", "d2b-panel-round", "scripts");
    mkdirSync(cliLifecycleDir, { recursive: true });
    cpSync(LIFECYCLE_CLI, join(cliLifecycleDir, "panel-lifecycle.mjs"));
    cpSync(
      join(fileURLToPath(new URL(".", import.meta.url)), "..", "..", ".github", "skills", "d2b-panel-round", "selection-table.json"),
      join(cliRoot, ".github", "skills", "d2b-panel-round", "selection-table.json"),
    );
    mkdirSync(join(cliRoot, ".github", "agents"), { recursive: true });
    for (const seat of [
      "software", "test", "product", "docs", "security", "observability",
      "simplicity", "reliability", "agentic", "nixos",
    ]) {
      writeFileSync(join(cliRoot, ".github", "agents", `panel-${seat}.agent.md`), `name: ${seat}\n`);
    }
    execFileSync("git", ["init", "--quiet"], { cwd: cliRoot });
    execFileSync("git", ["config", "user.name", "d2b test"], { cwd: cliRoot });
    execFileSync("git", ["config", "user.email", "d2b-test@example.invalid"], { cwd: cliRoot });
    writeFileSync(join(cliRoot, "base.txt"), "base\n");
    execFileSync("git", ["add", "."], { cwd: cliRoot });
    execFileSync("git", ["commit", "--quiet", "-m", "base"], { cwd: cliRoot });
    const cliBase = execFileSync("git", ["rev-parse", "HEAD"], {
      cwd: cliRoot,
      encoding: "utf8",
    }).trim();
    mkdirSync(join(cliRoot, "src"), { recursive: true });
    writeFileSync(join(cliRoot, "src", "panel.js"), "candidate\n");
    execFileSync("git", ["add", "src/panel.js"], { cwd: cliRoot });
    execFileSync("git", ["commit", "--quiet", "-m", "candidate"], { cwd: cliRoot });
    const cliFirstTip = execFileSync("git", ["rev-parse", "HEAD"], {
      cwd: cliRoot,
      encoding: "utf8",
    }).trim();
    const cliCandidate = join(cliRoot, "candidate.json");
    const cliDiscoveryVerdicts = join(cliRoot, "discovery-verdicts.json");
    const cliDiscoveryResults = join(cliRoot, "discovery-results.json");
    const cliGroups = join(cliRoot, "groups.json");
    const cliLedger = join(cliRoot, "ledger.json");
    const cliResponses = join(cliRoot, "responses.json");
    const cliSelf = join(cliRoot, "self.json");
    const cliDelta = join(cliRoot, "actual-delta.json");
    const cliVerificationSelectionCandidate = join(cliRoot, "verification-candidate.json");
    const cliFirstRound = join(cliRoot, ".scratch", "panel", "spec004w1-r1");
    const cliRound = join(cliRoot, ".scratch", "panel", "spec004w1-r2");
    const cliVerificationResults = join(cliRound, "verification-results.json");
    const cliApproval = join(cliRound, "approval.json");
    const cliMetrics = join(cliRound, "metrics.json");
    const cliEvidence = join(cliRoot, "finalized-evidence.md");
    const cliDiscoveryReviewerNotes = join(
      cliRoot,
      "finalized-discovery-reviewer-notes",
    );
    const cliVerificationReviewerNotes = join(
      cliRoot,
      "finalized-verification-reviewer-notes",
    );
    const cliMakeRecords = join(
      fileURLToPath(new URL(".", import.meta.url)),
      "..",
      "..",
      ".github",
      "skills",
      "d2b-panel-round",
      "scripts",
      "make-records.mjs",
    );
    const address = (snapshot, content = "cli-content") => ({
      program: "SPEC004",
      wave: "spec004w1",
      candidate_id: "cli-candidate",
      content_id: content,
      snapshot_sha256: snapshot,
      changed_paths: ["src/panel.js"],
    });
    writeFileSync(cliCandidate, stableStringify(address("d".repeat(64))));
    const runCli = (...args) =>
      execFileSync("node", [LIFECYCLE_CLI, ...args], {
        cwd: cliRoot,
        encoding: "utf8",
      }).trim();
    const writeFinalizedReviewerNotes = (directory, selectionPath, phase) => {
      const roster = JSON.parse(readFileSync(selectionPath, "utf8")).roster;
      mkdirSync(directory);
      for (const seat of roster) {
        writeFileSync(
          join(directory, `${seat}.md`),
          `# Reviewer notes for ${seat}\n\n${phase} notes finalized before staging.\n`,
        );
      }
      return roster;
    };
    const discoverySelection = runCli("select", cliCandidate, "spec004w1");
    const discoveryRequest = join(cliRoot, "discovery-request.json");
    runCli("discovery-request", discoverySelection, cliCandidate, discoveryRequest);
    writeFileSync(
      cliEvidence,
      "# Finalized validation evidence\n\n- Focused public CLI fixture: PASS\n",
    );
    const discoveryRoster = writeFinalizedReviewerNotes(
      cliDiscoveryReviewerNotes,
      discoverySelection,
      "Discovery",
    );
    const staged = spawnSync("bash", [
      cliStageScript,
      cliBase,
      cliBase,
      "spec004w1-r1",
      "--selection",
      discoverySelection,
      "--candidate",
      cliCandidate,
      "--discovery-request",
      discoveryRequest,
      "--evidence",
      cliEvidence,
      "--reviewer-notes-dir",
      cliDiscoveryReviewerNotes,
    ], { cwd: cliRoot, encoding: "utf8" });
    check(
      "CLI integration reaches finalized staged review evidence",
      staged.status === 0 &&
        existsSync(join(cliFirstRound, "review-request.md")) &&
        existsSync(join(cliFirstRound, ".complete")) &&
        readFileSync(join(cliFirstRound, "evidence.md"), "utf8") ===
          readFileSync(cliEvidence, "utf8") &&
        discoveryRoster.every((seat) =>
          readFileSync(
            join(cliFirstRound, "reviewer-notes", `${seat}.md`),
            "utf8",
          ) === readFileSync(
            join(cliDiscoveryReviewerNotes, `${seat}.md`),
            "utf8",
          )),
      `${staged.stdout}${staged.stderr}`,
    );
    const stagedDiscoveryRequest = spawnSync("bash", [
      cliStageScript,
      cliBase,
      cliBase,
      "spec004w1-r1",
      "--selection",
      discoverySelection,
      "--candidate",
      cliCandidate,
      "--discovery-request",
      discoveryRequest,
      "--evidence",
      cliEvidence,
      "--reviewer-notes-dir",
      cliDiscoveryReviewerNotes,
    ], { cwd: cliRoot, encoding: "utf8" });
    const discoveryRequestBindsEvidence = () => {
      const source = JSON.parse(readFileSync(discoveryRequest, "utf8"));
      const actual = JSON.parse(
        readFileSync(join(cliFirstRound, "discovery-request.json"), "utf8"),
      );
      const evidenceBytes = readFileSync(cliEvidence, "utf8");
      const expectedDescriptor = {
        artifact_kind: "d2b-panel/validation-evidence",
        path: "evidence.md",
        sha256: sha256(evidenceBytes),
        size_bytes: Buffer.byteLength(evidenceBytes),
      };
      return JSON.stringify({
        ...actual,
        validation_evidence: source.validation_evidence,
      }) === JSON.stringify(source) &&
        actual.validation_evidence.length ===
          source.validation_evidence.length + 1 &&
        actual.validation_evidence.some((entry) =>
          JSON.stringify(entry) === JSON.stringify(expectedDescriptor));
    };
    check(
      "CLI stage preserves the discovery request and binds finalized evidence",
      stagedDiscoveryRequest.status === 0 &&
        discoveryRequestBindsEvidence(),
      `${stagedDiscoveryRequest.stdout}${stagedDiscoveryRequest.stderr}`,
    );
    writeFileSync(
      cliDiscoveryVerdicts,
      stableStringify(Object.fromEntries(discoveryRoster.map((seat) => [seat, {
        engineer: seat,
        signoff: seat !== "software",
        summary: seat === "software"
          ? "A source mapping issue was found."
          : "No discovery findings.",
        recommendations: seat === "software"
          ? [{
              severity: "high",
              where: "scripts/panel.js:1",
              what: "A source can disappear.",
              why: "The ledger would be incomplete.",
              fix: "Validate source coverage.",
            }]
          : [],
      }]))),
    );
    runCli("adapt-discovery", cliDiscoveryVerdicts, cliDiscoveryResults);
    const discoveryVerdictObjects = JSON.parse(readFileSync(cliDiscoveryVerdicts, "utf8"));
    for (const seat of discoveryRoster) {
      writeFileSync(
        join(cliFirstRound, "verdicts", `${seat}.json`),
        stableStringify(discoveryVerdictObjects[seat]),
      );
    }
    writeFileSync(
      cliGroups,
      stableStringify([{
        source_finding_ids: ["software:1"],
        description: "A source can disappear.",
        severity: "MAJOR",
        impact: "The ledger would be incomplete.",
        recommendation: "Validate source coverage.",
      }]),
    );
    runCli("merge-ledger", discoverySelection, cliDiscoveryResults, cliGroups, cliLedger);
    runCli("response-template", cliLedger, cliResponses);
    const cliResponseObject = JSON.parse(readFileSync(cliResponses, "utf8"));
    cliResponseObject.responses[0] = {
      issue_id: "R1",
      disposition: "Fixed",
      changed_surface: ["src/panel.js"],
      justification: "The source mapping is now validated.",
      evidence: "focused Node test",
    };
    writeFileSync(cliResponses, stableStringify(cliResponseObject));
    writeFileSync(cliSelf, stableStringify({
      tests: "passed",
      lint: "passed",
      formatting: "passed",
      static_analysis: "passed",
      build: "not applicable",
      uncovered_areas: "none",
      self_review: "passed",
    }));
    writeFileSync(cliDelta, stableStringify({
      changed_paths: ["src/panel.js"],
      diff: "the focused fix",
    }));
    writeFileSync(join(cliRoot, "src", "panel.js"), "candidate fixed\n");
    execFileSync("git", ["add", "src/panel.js"], { cwd: cliRoot });
    execFileSync("git", ["commit", "--quiet", "-m", "fix"], { cwd: cliRoot });
    writeFileSync(
      cliVerificationSelectionCandidate,
      stableStringify(address("e".repeat(64), "cli-current-content")),
    );
    const verificationSelection = runCli(
      "select",
      cliVerificationSelectionCandidate,
      "spec004w1",
      "--phase",
      "verification",
      "--previous-selection",
      discoverySelection,
      "--fix-delta",
      cliDelta,
    );
    const cliVerificationRequests = join(cliRoot, "verification-requests");
    runCli(
      "verification",
      verificationSelection,
      cliLedger,
      cliResponses,
      cliSelf,
      cliVerificationRequests,
      "--candidate",
      cliVerificationSelectionCandidate,
      "--prior-selection",
      discoverySelection,
      "--prior-verdicts",
      join(cliFirstRound, "verdicts"),
      "--delta",
      cliDelta,
    );
    const verificationRoster = writeFinalizedReviewerNotes(
      cliVerificationReviewerNotes,
      verificationSelection,
      "Verification",
    );
    const stagedVerification = spawnSync("bash", [
      cliStageScript,
      cliBase,
      cliFirstTip,
      "spec004w1-r2",
      "--selection",
      verificationSelection,
      "--candidate",
      cliVerificationSelectionCandidate,
      "--ledger",
      cliLedger,
      "--responses",
      cliResponses,
      "--self-verification",
      cliSelf,
      "--verification-dir",
      cliVerificationRequests,
      "--evidence",
      cliEvidence,
      "--reviewer-notes-dir",
      cliVerificationReviewerNotes,
    ], { cwd: cliRoot, encoding: "utf8" });
    check(
      "CLI verification stage materializes exact canonical artifacts",
      stagedVerification.status === 0 &&
        readFileSync(join(cliRound, "selection.json"), "utf8") ===
          readFileSync(verificationSelection, "utf8") &&
        readFileSync(join(cliRound, "current-candidate.json"), "utf8") ===
          readFileSync(cliVerificationSelectionCandidate, "utf8") &&
        readFileSync(join(cliRound, "discovery-ledger.json"), "utf8") ===
          readFileSync(cliLedger, "utf8") &&
        readFileSync(join(cliRound, "responses.json"), "utf8") ===
          readFileSync(cliResponses, "utf8") &&
        readFileSync(join(cliRound, "self-verification.json"), "utf8") ===
          readFileSync(cliSelf, "utf8") &&
        readFileSync(join(cliRound, "evidence.md"), "utf8") ===
          readFileSync(cliEvidence, "utf8") &&
        verificationRoster.every((seat) =>
          readFileSync(
            join(cliRound, "reviewer-notes", `${seat}.md`),
            "utf8",
          ) === readFileSync(
            join(cliVerificationReviewerNotes, `${seat}.md`),
            "utf8",
          )) &&
        existsSync(join(cliRound, ".complete")),
      `${stagedVerification.stdout}${stagedVerification.stderr}`,
    );
    const missingVerificationFlags = spawnSync("node", [
      LIFECYCLE_CLI,
      "verification",
      verificationSelection,
      cliLedger,
      cliResponses,
      cliSelf,
      join(cliRoot, "verification-missing-flags"),
    ], { cwd: cliRoot, encoding: "utf8" });
    check(
      "verification CLI refuses empty candidate, prior, verdict, and delta defaults",
      missingVerificationFlags.status !== 0 &&
        /requires --candidate, --prior-selection, --prior-verdicts, and --delta/.test(
          `${missingVerificationFlags.stdout}${missingVerificationFlags.stderr}`,
        ),
    );
    const ledgerIssues = JSON.parse(readFileSync(cliLedger, "utf8")).issues;
    for (const seat of verificationRoster) {
      writeFileSync(join(cliRound, "verdicts", `${seat}.json`), stableStringify({
        engineer: seat,
        signoff: true,
        summary: "Verification passed.",
        issue_statuses: Object.fromEntries(ledgerIssues.map((issue) => [issue.id, "resolved"])),
        recommendations: [],
      }));
    }
    runCli(
      "adapt-verification",
      cliLedger,
      join(cliRound, "verdicts"),
      join(cliRound, "verification-results.json"),
      "--selection",
      join(cliRound, "selection.json"),
      "--candidate",
      join(cliRound, "current-candidate.json"),
    );
    check(
      "adapt-verification reads one staged verdict per selected seat in roster order",
      JSON.parse(readFileSync(join(cliRound, "verification-results.json"), "utf8"))
        .results.map((result) => result.seat).join(",") === verificationRoster.join(","),
    );
    runCli(
      "verification",
      join(cliRound, "selection.json"),
      join(cliRound, "discovery-ledger.json"),
      join(cliRound, "responses.json"),
      join(cliRound, "self-verification.json"),
      join(cliRound, "verification"),
      "--candidate",
      join(cliRound, "current-candidate.json"),
      "--prior-selection",
      join(cliFirstRound, "selection.json"),
      "--prior-verdicts",
      join(cliFirstRound, "verdicts"),
      "--delta",
      cliDelta,
    );
    const approvalResult = spawnSync("node", [
      LIFECYCLE_CLI,
      "approval",
      join(cliRound, "selection.json"),
      join(cliRound, "discovery-ledger.json"),
      join(cliRound, "responses.json"),
      join(cliRound, "verification-results.json"),
      join(cliRound, "approval.json"),
      "--candidate",
      join(cliRound, "current-candidate.json"),
    ], { cwd: cliRoot, encoding: "utf8" });
    check(
      "approval CLI exposes a passing artifact",
      approvalResult.status === 0 &&
        JSON.parse(readFileSync(cliApproval, "utf8")).approved === true,
      `${approvalResult.stdout}${approvalResult.stderr}`,
    );
    runCli(
      "metrics",
      "--selection",
      join(cliRound, "selection.json"),
      "--ledger",
      join(cliRound, "discovery-ledger.json"),
      "--responses",
      join(cliRound, "responses.json"),
      "--verification-results",
      join(cliRound, "verification-results.json"),
      "--output",
      join(cliRound, "metrics.json"),
    );
    writeFileSync(join(cliRound, "observed.json"), stableStringify(
      Object.fromEntries(verificationRoster.map((seat, index) => [seat, {
        provider: "github-copilot",
        model: "gpt-5.6-sol",
        reasoning_effort: "xhigh",
        run_id: `cli-run-${index}`,
        receipt_locator: `github-copilot://cli/${index}`,
      }])),
    ));
    const recordsResult = spawnSync("node", [
      cliMakeRecords,
      cliRound,
      "--selection",
      join(cliRound, "selection.json"),
      "--ledger",
      join(cliRound, "discovery-ledger.json"),
      "--responses",
      join(cliRound, "responses.json"),
      "--verification-results",
      join(cliRound, "verification-results.json"),
      "--approval",
      join(cliRound, "approval.json"),
    ], { cwd: cliRoot, encoding: "utf8" });
    check(
      "make-records CLI publishes the complete record set",
      recordsResult.status === 0 &&
        verificationRoster.every((seat) =>
          existsSync(join(cliRound, "records", `${seat}.json`))),
      `${recordsResult.stdout}${recordsResult.stderr}`,
    );
    check(
      "metrics CLI writes deterministic canonical metrics",
      JSON.parse(readFileSync(cliMetrics, "utf8")).artifact_kind === "d2b-panel/metrics" &&
        JSON.parse(readFileSync(cliMetrics, "utf8")).status === "complete" &&
        JSON.parse(readFileSync(cliMetrics, "utf8")).degraded === false &&
        JSON.parse(readFileSync(cliMetrics, "utf8")).metrics.initial_unique_findings > 0 &&
        JSON.parse(readFileSync(cliMetrics, "utf8")).metrics.average_fixed_issues_per_implementation_iteration > 0,
    );
    const canonicalVerificationResultsBytes = readFileSync(cliVerificationResults);
    const tamperedMetricsVerification = JSON.parse(
      readFileSync(cliVerificationResults, "utf8"),
    );
    tamperedMetricsVerification.results[0].verified_issue_statuses.R1 = "accepted";
    writeFileSync(cliVerificationResults, stableStringify(tamperedMetricsVerification));
    const degradedMetrics = spawnSync("node", [
      LIFECYCLE_CLI,
      "metrics",
      "--selection",
      verificationSelection,
      "--ledger",
      cliLedger,
      "--responses",
      cliResponses,
      "--verification-results",
      cliVerificationResults,
      "--output",
      join(cliRoot, "tampered-metrics.json"),
    ], { cwd: cliRoot, encoding: "utf8" });
    check(
      "metrics refuses a non-passing verification status",
      degradedMetrics.status !== 0 &&
        /passing verification|status/.test(
          `${degradedMetrics.stdout}${degradedMetrics.stderr}`,
        ),
    );
    writeFileSync(cliVerificationResults, canonicalVerificationResultsBytes);
    check(
      "public CLI integration produces named lifecycle artifacts",
      readFileSync(discoveryRequest, "utf8").includes("comprehensive") &&
        existsSync(cliApproval) &&
        existsSync(cliMetrics),
    );
  } finally {
    rmSync(cliRoot, { recursive: true, force: true });
  }
} finally {
  rmSync(root, { recursive: true, force: true });
}

console.log("panel lifecycle: legacy continuation");
{
  const legacyRecords = [
    ["software", "[critical] unsafe source"],
    ["test", "[HIGH] missing negative"],
    ["nixos", "[medium] module wording"],
    ["networking", "[low] route name"],
    ["security", "critical without bracket"],
    ["rust", "[LoW] rust style"],
    ["product", "[criticality] not exact"],
    ["docs", " [high] leading space"],
    ["observability", "[medium] metric"],
    ["kernel", "[critical] syscall"],
  ].map(([role, recommendation], index) => ({
    role,
    output_sha256: String(index + 1).repeat(64).slice(0, 64),
    recommendations: [recommendation],
  }));
  const legacyCandidate = candidate({ changed_paths: ["Makefile"] });
  const legacyWireRecords = legacyRecords.map((record, index) => ({
    artifact_kind: "d2b-delivery/panel-receipt",
    schema_version: 2,
    role: record.role,
    candidate_id: legacyCandidate.candidate_id,
    content_id: legacyCandidate.content_id,
    snapshot_sha256: legacyCandidate.snapshot_sha256,
    model_version: "gemini-3.1-pro-preview",
    provider: "github-copilot",
    reasoning_effort: "high",
    run_id: `run-${record.role}-${index}`,
    receipt_locator: `github-copilot://runs/${record.role}/${index}`,
    output_sha256: record.output_sha256,
    signoff: false,
    recommendations: record.recommendations,
  }));
  const legacyRequest = {
    artifact_kind: "d2b-delivery/panel-request",
    schema_version: 2,
    program: legacyCandidate.program,
    wave: legacyCandidate.wave,
    candidate_id: legacyCandidate.candidate_id,
    content_id: legacyCandidate.content_id,
    snapshot_sha256: legacyCandidate.snapshot_sha256,
    provider: "github-copilot",
    model_version: "gemini-3.1-pro-preview",
    reasoning_effort: "high",
    roles: [
      "software", "test", "nixos", "networking", "security",
      "rust", "product", "docs", "observability", "kernel",
    ],
    record_artifact_kind: "d2b-delivery/panel-receipt",
    record_schema_version: 2,
    record_files: [
      "software.json", "test.json", "nixos.json", "networking.json",
      "security.json", "rust.json", "product.json", "docs.json",
      "observability.json", "kernel.json",
    ],
  };
  const legacyAttestation = {
    roles: legacyRequest.roles,
    records: legacyWireRecords.map((record) => ({
      role: record.role,
      file: `${record.role}.json`,
      sha256: record.output_sha256,
      run_id: record.run_id,
    })),
    unanimous: false,
  };
  const legacyBundle = {
    request: legacyRequest,
    records: legacyWireRecords,
    attestation: legacyAttestation,
    seal_panel: legacyAttestation,
  };
  const imported = importLegacyRound(
    legacyBundle,
    {
      candidate: legacyCandidate,
    },
  );
  check(
    "object legacy ten-seat import stays partial without exact bytes",
    imported.complete === false &&
      imported.discovery_required === true &&
      imported.discovery_mode === "run-one-current-discovery",
  );
  check("legacy source identity includes digest, seat, and ordinal", imported.sources[0].source_id.includes("software:1"));
  check("legacy raw recommendation text is preserved", imported.sources[0].raw_text === "[critical] unsafe source");
  check("legacy attribution is preserved", imported.sources[0].raw_attribution === "software");
  check("exact critical prefix maps to BLOCKER", imported.sources.find((source) => source.seat === "software").severity === "BLOCKER");
  check("exact high prefix maps to MAJOR", imported.sources.find((source) => source.seat === "test").severity === "MAJOR");
  check("exact medium prefix maps to MINOR", imported.sources.find((source) => source.seat === "nixos").severity === "MINOR");
  check("exact low prefix maps to NIT", imported.sources.find((source) => source.seat === "rust").severity === "NIT");
  check("unbracketed or non-exact prefix maps to migration MAJOR", imported.sources.find((source) => source.seat === "security").migration_assigned_severity === true);
  check("legacy rust maps responsibility to software Rust profile", imported.responsibilities.some((item) => item.legacy_seat === "rust" && item.current_seat === "software" && item.profile === "rust"));
  check("legacy rust profile is bound to current software", imported.profiles.software.includes("rust"));
  check("current build selection widens the imported roster", imported.lifecycle_roster.includes("build"));
  const exactLegacyRecordBytes = Object.fromEntries(
    legacyWireRecords.map((record) => [record.role, stableStringify(record)]),
  );
  const exactLegacyAttestation = {
    roles: legacyRequest.roles,
    records: legacyWireRecords.map((record) => ({
      role: record.role,
      file: `${record.role}.json`,
      sha256: sha256(stableStringify(record)),
      run_id: record.run_id,
    })),
    unanimous: false,
  };
  const exactObjectImport = importLegacyRound(
    {
      request: legacyRequest,
      records: legacyWireRecords,
      record_bytes: exactLegacyRecordBytes,
      attestation: exactLegacyAttestation,
    },
    { candidate: legacyCandidate },
  );
  check(
    "object legacy import is complete only with exact record bytes and attestation digests",
    exactObjectImport.complete === true &&
      exactObjectImport.discovery_required === false,
  );
  const exactPartialRecords = legacyWireRecords.slice(0, 2);
  const exactPartialImport = importLegacyRound(
    {
      request: legacyRequest,
      records: exactPartialRecords,
      record_bytes: Object.fromEntries(
        exactPartialRecords.map((record) => [record.role, stableStringify(record)]),
      ),
    },
    { candidate: legacyCandidate },
  );
  check(
    "exact partial legacy records remain an incomplete import",
    exactPartialImport.complete === false &&
      exactPartialImport.discovery_required === true &&
      exactPartialImport.sources.length === 2,
  );
  const exactWithoutAttestation = importLegacyRound(
    {
      request: legacyRequest,
      records: legacyWireRecords,
      record_bytes: exactLegacyRecordBytes,
    },
    { candidate: legacyCandidate },
  );
  check(
    "exact ten records without attestation remain incomplete",
    exactWithoutAttestation.complete === false &&
      exactWithoutAttestation.discovery_required === true &&
      exactWithoutAttestation.sources.length === legacyWireRecords.length,
  );
  rejects(
    "a malformed supplied legacy proof is rejected even for partial records",
    () => importLegacyRound({
      request: legacyRequest,
      records: exactPartialRecords,
      record_bytes: Object.fromEntries(
        exactPartialRecords.map((record) => [record.role, stableStringify(record)]),
      ),
      attestation: {
        roles: ["software"],
        records: [],
        unanimous: false,
      },
    }, { candidate: legacyCandidate }),
    /legacy panel attestation|fixed-ten|exactly ten/,
  );
  const importedWithPriorSelection = importLegacyRound(
    legacyBundle,
    {
      selection: priorSelection,
      candidate: legacyCandidate,
    },
  );
  check(
    "legacy import widens a prior selection when the current candidate triggers build",
    importedWithPriorSelection.lifecycle_roster.includes("build"),
  );
  const importedAgain = importLegacyRound(
    legacyBundle,
    { candidate: legacyCandidate },
  );
  check("repeated legacy import has stable bytes", stableStringify(imported) === stableStringify(importedAgain));
  const legacyDir = mkdtempSync(join(tmpdir(), "d2b-legacy-round-"));
  try {
    mkdirSync(join(legacyDir, "records"));
    writeFileSync(join(legacyDir, "panel-request.json"), stableStringify(legacyRequest));
    for (const record of legacyWireRecords) {
      writeFileSync(join(legacyDir, "records", `${record.role}.json`), stableStringify(record));
    }
    writeFileSync(
      join(legacyDir, "attestation.json"),
      stableStringify({
        roles: legacyRequest.roles,
        records: legacyWireRecords.map((record) => ({
          role: record.role,
          file: `${record.role}.json`,
          sha256: sha256(stableStringify(record)),
          run_id: record.run_id,
        })),
        unanimous: false,
      }),
    );
    const directoryImport = importLegacyRound(legacyDir, {
      candidate: legacyCandidate,
    });
    check("legacy directory import reads its fixed-ten records directory", directoryImport.sources.length === 10);
    writeFileSync(
      join(legacyDir, "records", "software.json"),
      stableStringify({
        ...legacyWireRecords[0],
        recommendations: ["tampered"],
      }),
    );
    try {
      importLegacyRound(legacyDir, { candidate: legacyCandidate });
      check("legacy directory import binds attestation to exact record bytes", false);
    } catch (cause) {
      check(
        "legacy directory import binds attestation to exact record bytes",
        /exact bytes|digest does not match/.test(cause.message),
        cause.message,
      );
    }
  } finally {
    rmSync(legacyDir, { recursive: true, force: true });
  }

  const partial = importLegacyRound(
    { records: legacyWireRecords.slice(0, 2) },
    { candidate: legacyCandidate },
  );
  check("partial legacy import retains completed sources", partial.sources.length === 2);
  check("partial legacy import requests one current discovery", partial.discovery_required === true && partial.discovery_mode === "run-one-current-discovery");
  rejects(
    "current panel format is not accepted as legacy",
    () => importLegacyRound({ records: [{ ...legacyWireRecords[0], panel_format_version: 1 }] }),
    /current format|legacy fallback/,
  );
  rejects(
    "duplicate legacy role is refused",
    () => importLegacyRound({ records: [legacyWireRecords[0], legacyWireRecords[0]] }),
    /duplicate record/,
  );
}

if (failures > 0) {
  console.error(`\ntest-panel-lifecycle: ${failures} failure(s)`);
  process.exit(1);
}
console.log("\ntest-panel-lifecycle: all cases passed");
