#!/usr/bin/env node
// Focused behavior coverage for the standard Copilot panel lifecycle.

import {
  appendLateFindings,
  advanceVerification,
  adaptDiscoveryVerdict,
  adaptVerificationVerdict,
  calculateMetrics,
  changedPathsFromGitRange,
  createApprovalArtifact,
  createDiscoveryRequest,
  createDiscoveryResultArtifact,
  createResponseTemplate,
  createSelection,
  createVerificationResultArtifact,
  continueLegacyImport,
  evaluateApproval,
  importLegacyRound,
  lateFindingAdmission,
  LATE_FINDING_SCHEMA,
  mergeDiscoveryLedger,
  prepareVerification,
  readSelection,
  readSelectionTable,
  selectRoster,
  selectLifecycleRoster,
  sha256,
  stableStringify,
  validateDiscoveryResults,
  validateDiscoveryResultArtifact,
  validateVerificationResultArtifact,
  validateCandidateAgainstSelection,
  validateFixScope,
  validateMonotonicRoster,
  validateSelectionAgainstTable,
  validateResponses,
  validateSelection,
  validateSelfVerification,
  validateVerificationRequest,
  validateVerificationResults,
  writeVerificationArtifacts,
  writeAdvanceVerification,
  writeDirectoryCreateOrCompare,
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
const RUST_SELECTION_FIXTURE = join(
  REPOSITORY_ROOT,
  "packages",
  "xtask",
  "src",
  "delivery",
  "testdata",
  "panel-selection-js.json",
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

function concurrentDirectoryPublish(directory, helperPath, bytePair) {
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
  return new Promise((resolvePromise) => {
    const results = [];
    const finish = () => {
      if (results.length === bytePair.length) {
        resolvePromise(results.sort((left, right) => left.index - right.index));
      }
    };
    for (const [index, bytes] of bytePair.entries()) {
      const child = spawn(
        process.execPath,
        ["--input-type=module", "-e", source, helperPath, directory, bytes],
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
  const documentedLines = documented.split(/\r?\n/);
  const unsupportedAdvanceLifecycle = documentedLines.some((line, index) =>
    line.includes("advance-verification") &&
    documentedLines
      .slice(index, index + 8)
      .join(" ")
      .includes("--lifecycle <lifecycle-id>"),
  );
  check(
    `${documentedPath.split("/").at(-1)} does not copy --lifecycle to advance-verification`,
    !unsupportedAdvanceLifecycle,
  );
}

function candidate(overrides = {}) {
  return {
    program: "SPEC004",
    wave: "spec004w1",
    candidate_id: "b".repeat(64),
    content_id: "c".repeat(64),
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

function adaptedDiscoveryArtifact(
  selection,
  results,
  currentCandidate = candidate(),
  selectionBytes = stableStringify(selection),
) {
  return {
    artifact_kind: "d2b-panel/discovery-result",
    schema_version: 1,
    phase: "discovery",
    lifecycle_id: selection.lifecycle_id,
    selection_sha256: sha256(selectionBytes),
    current_candidate: {
      program: currentCandidate.program,
      wave: currentCandidate.wave,
      candidate_id: currentCandidate.candidate_id,
      content_id: currentCandidate.content_id,
      snapshot_sha256: currentCandidate.snapshot_sha256,
    },
    results: selection.roster.map((seat) => ({
      ...results[seat],
      findings: results[seat].findings.map((finding) => ({
        ...finding,
        seat,
      })),
    })),
  };
}

function makeSelection(root, overrides = {}) {
  return createSelection(
    {
      ...candidate(overrides),
      lifecycle_id: overrides.lifecycle_id ?? "spec004w1",
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
    candidate_id: "b".repeat(64),
    content_id: "c".repeat(64),
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
  const rustSelection = createSelection(
    {
      program: "ADR046",
      wave: "W0",
      candidate_id:
        "b14017729b4312480b217eb378d89da8e629ca0ccb082a66230a297450d9270e",
      content_id:
        "2a7104db2b2fb08db7062bf0080594f4c002abede9328b579fee4700725a1c40",
      snapshot_sha256:
        "c6364dcb391510507604fbd15d5cc8fba1617ede694403fcb2296332cecd04c8",
      changed_paths: ["packages/xtask/src/delivery/panel.rs"],
      signals: [],
      candidate_class: "code",
      ambiguous: false,
      lifecycle_id: "rust-panel-interop",
      phase: "discovery",
    },
    { root },
  );
  check(
    "Rust panel-request selection fixture is exact JavaScript producer output",
    readFileSync(rustSelection.path, "utf8") ===
      readFileSync(RUST_SELECTION_FIXTURE, "utf8"),
  );

  const initial = makeSelection(root);
  priorSelection = initial.selection;
  check(
    "selection is rendered at the candidate-bound lifecycle address",
    initial.path.endsWith(
      "/.scratch/panel/spec004w1/selections/" +
        `${"b".repeat(64)}/` +
      `${"a".repeat(64)}.json`,
    ),
  );
  check("selection schema version is one", initial.selection.schema_version === 1);
  check("selection table version is two", initial.selection.selection_table_version === 2);
  check(
    "selection is readable after rendering",
    readSelection(initial.path).candidate_id === "b".repeat(64),
  );
  rejects(
    "selection producer rejects a non-digest candidate identifier",
    () => makeSelection(root, { candidate_id: "candidate-1" }),
    /candidate_id.*64-character hexadecimal/,
  );
  rejects(
    "selection producer rejects a non-digest content identifier",
    () => makeSelection(root, { content_id: "content-1" }),
    /content_id.*64-character hexadecimal/,
  );
  rejects(
    "selection producer mirrors qualified program and wave validation",
    () => makeSelection(root, { wave: "otherw1" }),
    /disagrees with candidate program|candidate wave/,
  );
  rejects(
    "selection producer applies the Rust bounded lifecycle rule",
    () => makeSelection(root, { lifecycle_id: "x".repeat(4097) }),
    /at most 4096 bytes/,
  );
  const duplicateProfileSelection = {
    ...initial.selection,
    profiles: {
      ...initial.selection.profiles,
      software: [...initial.selection.profiles.software, "javascript"],
    },
  };
  rejects(
    "selection producer rejects duplicate profiles",
    () => validateSelection(duplicateProfileSelection),
    /repeats profile/,
  );
  rejects(
    "one lifecycle cannot create a second discovery selection",
    () => createSelection({
      ...candidate({ snapshot_sha256: "9".repeat(64) }),
      lifecycle_id: "spec004w1",
      phase: "discovery",
    }, { root }),
    /already has a discovery selection/,
  );
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
  const floorHole = {
    ...initial.selection,
    roster: [
      ...initial.selection.roster.filter((seat) => seat !== "reliability"),
      "build",
    ],
    profiles: {
      ...Object.fromEntries(
        Object.entries(initial.selection.profiles)
          .filter(([seat]) => seat !== "reliability"),
      ),
      build: [],
    },
  };
  rejects(
    "selection validation requires canonical deterministic floor fill",
    () => validateSelectionAgainstTable(floorHole),
    /canonical floor-fill seat.*reliability/,
  );
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
      lifecycle_id: "spec004w1-backslash",
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
  const actualDiscoveryVerdicts = Object.fromEntries(
    initial.selection.roster.map((seat) => [
      seat,
      seat === "software"
        ? actualDiscoveryVerdict
        : {
            engineer: seat,
            signoff: true,
            summary: "No findings.",
            recommendations: [],
          },
    ]),
  );
  const adaptedDiscoveryOutput = createDiscoveryResultArtifact({
    selection: initial.selection,
    selection_bytes: stableStringify(initial.selection),
    candidate: candidate(),
    verdicts: actualDiscoveryVerdicts,
  });
  check(
    "discovery adapter output binds lifecycle, exact selection bytes, and candidate",
    adaptedDiscoveryOutput.lifecycle_id === initial.selection.lifecycle_id &&
      adaptedDiscoveryOutput.selection_sha256 ===
        sha256(stableStringify(initial.selection)) &&
      adaptedDiscoveryOutput.current_candidate.snapshot_sha256 ===
        candidate().snapshot_sha256 &&
      adaptedDiscoveryOutput.results[0].complete === true,
  );
  check(
    "strict discovery artifact validation accepts canonical adapter output",
    validateDiscoveryResultArtifact(adaptedDiscoveryOutput, {
      selection: initial.selection,
      selection_bytes: stableStringify(initial.selection),
      candidate: candidate(),
    }).length === 1,
  );
  rejects(
    "adapted discovery output refuses undeclared result fields",
    () => validateDiscoveryResultArtifact({
      ...adaptedDiscoveryOutput,
      results: adaptedDiscoveryOutput.results.map((result, index) => index === 0
        ? { ...result, summary: "Adapter output must stay canonical." }
        : result),
    }, {
      selection: initial.selection,
      selection_bytes: stableStringify(initial.selection),
      candidate: candidate(),
    }),
    /fields.*summary.*expected exactly/,
  );
  rejects(
    "an inconsistent actual discovery signoff is refused",
    () => adaptDiscoveryVerdict({ ...actualDiscoveryVerdict, signoff: true }),
    /signoff/,
  );
  rejects(
    "a discovery reviewer verdict refuses undeclared top-level fields",
    () => adaptDiscoveryVerdict({
      ...actualDiscoveryVerdict,
      complete: true,
    }),
    /fields.*complete.*expected exactly/,
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
  const discoveryArtifact = adaptedDiscoveryArtifact(
    initial.selection,
    withFinding,
  );
  const discoveryMergeInput = {
    selection: initial.selection,
    selection_bytes: stableStringify(initial.selection),
    candidate: candidate(),
    discovery_results: discoveryArtifact,
  };
  const ledger = mergeDiscoveryLedger({
    ...discoveryMergeInput,
    groups,
  });
  check("deduplication creates one stable R identifier", ledger.issues[0].id === "R1");
  check("deduplication preserves both source attributions", ledger.issues[0].source_finding_ids.join(",") === "software:1,test:1");
  const ledgerAgain = mergeDiscoveryLedger({
    ...discoveryMergeInput,
    groups,
  });
  check("identical ledger inputs are byte-stable", stableStringify(ledger) === stableStringify(ledgerAgain));
  rejects(
    "merge refuses stale discovery results when exact selection bytes differ",
    () => mergeDiscoveryLedger({
      ...discoveryMergeInput,
      selection_bytes: `${stableStringify(initial.selection)}\n`,
      groups,
    }),
    /not bound to the exact selection bytes/,
  );
  const staleCandidate = candidate({
    candidate_id: "d".repeat(64),
    content_id: "e".repeat(64),
    snapshot_sha256: "b".repeat(64),
  });
  const staleCandidateSelection = {
    ...initial.selection,
    candidate_id: staleCandidate.candidate_id,
    content_id: staleCandidate.content_id,
    snapshot_sha256: staleCandidate.snapshot_sha256,
  };
  check(
    "stale candidate negative keeps the same roster",
    staleCandidateSelection.roster.join(",") ===
      initial.selection.roster.join(","),
  );
  const staleCandidateSelectionBytes = stableStringify(staleCandidateSelection);
  rejects(
    "merge refuses stale same-roster discovery results for another candidate",
    () => mergeDiscoveryLedger({
      selection: staleCandidateSelection,
      selection_bytes: staleCandidateSelectionBytes,
      candidate: staleCandidate,
      discovery_results: {
        ...discoveryArtifact,
        selection_sha256: sha256(staleCandidateSelectionBytes),
      },
      groups,
    }),
    /current_candidate.*does not match|current_candidate.*disagrees/,
  );
  const staleLifecycleSelection = createSelection({
    ...candidate(),
    lifecycle_id: "spec004w1-stale",
    phase: "discovery",
  }, { root: join(root, "stale-lifecycle-selection") }).selection;
  check(
    "stale lifecycle negative keeps the same roster",
    staleLifecycleSelection.roster.join(",") ===
      initial.selection.roster.join(","),
  );
  const staleLifecycleSelectionBytes = stableStringify(staleLifecycleSelection);
  rejects(
    "merge refuses stale same-roster discovery results from another lifecycle",
    () => mergeDiscoveryLedger({
      selection: staleLifecycleSelection,
      selection_bytes: staleLifecycleSelectionBytes,
      candidate: candidate(),
      discovery_results: {
        ...discoveryArtifact,
        selection_sha256: sha256(staleLifecycleSelectionBytes),
      },
      groups,
    }),
    /lifecycle_id disagrees/,
  );
  rejects(
    "an unmapped source finding is refused",
    () => mergeDiscoveryLedger({
      ...discoveryMergeInput,
      groups: [{ source_finding_ids: ["software:1"] }],
    }),
    /mapping is incomplete/,
  );
  rejects(
    "a source mapped into two groups is refused",
    () => mergeDiscoveryLedger({
      ...discoveryMergeInput,
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
      ...discoveryMergeInput,
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
  const fixedMajor = {
    issue_id: "R1",
    disposition: "Fixed",
    changed_surface: ["src/panel.js"],
    justification: "The MAJOR was fixed.",
    evidence: "focused test",
  };
  check(
    "fixed MAJOR is approvable",
    evaluateApproval({
      selection: initial.selection,
      ledger: majorLedger,
      responses: [fixedMajor],
      verification_results: allVerificationResults(
        initial.selection.roster,
        {},
        majorLedger,
      ),
    }).approved === true,
  );
  const rejectedBlocker = {
    issue_id: "R1",
    disposition: "Intentionally rejected",
    justification: "The BLOCKER is intentionally rejected for this process test.",
  };
  const blockerLedger = {
    ...majorLedger,
    sources: [{ ...majorLedger.sources[0], severity: "BLOCKER" }],
    issues: [{ ...majorLedger.issues[0], severity: "BLOCKER" }],
  };
  check(
    "Intentionally rejected BLOCKER remains blocking",
    evaluateApproval({
      selection: initial.selection,
      ledger: blockerLedger,
      responses: [rejectedBlocker],
      verification_results: allVerificationResults(
        initial.selection.roster,
        {},
        blockerLedger,
      ),
    }).approved === false,
  );
  rejects(
    "unverified Withdrawn MAJOR is refused before approval",
    () => validateResponses(majorLedger, [{
      issue_id: "R1",
      disposition: "Withdrawn",
      justification: "The report was withdrawn.",
      evidence: "source inspection",
    }]),
    /verified_factual_status/,
  );
  const repositoryAcceptedMajor = {
    ...acceptedMajor,
    acceptance: {
      accepter: "repository-maintainer",
      capacity: "repository maintainer",
      justification: "The maintainer accepts the documented residual risk.",
    },
  };
  check(
    "repository-maintainer acceptance approves Deferred MAJOR",
    evaluateApproval({
      selection: initial.selection,
      ledger: majorLedger,
      responses: [repositoryAcceptedMajor],
      verification_results: allVerificationResults(
        initial.selection.roster,
        {},
        majorLedger,
      ),
    }).approved === true,
  );
  const acceptanceMutations = [
    undefined,
    null,
    [],
    "accepted",
    { capacity: "merge owner", justification: "x" },
    { accepter: "x", justification: "x" },
    { accepter: "x", capacity: "merge owner" },
    { accepter: "x", capacity: "merge owner", justification: "x", extra: "no" },
    { accepter: 1, capacity: "merge owner", justification: "x" },
    { accepter: "x", capacity: 1, justification: "x" },
    { accepter: "x", capacity: "merge owner", justification: 1 },
    { accepter: "", capacity: "merge owner", justification: "x" },
    { accepter: " ", capacity: "merge owner", justification: "x" },
    { accepter: "x", capacity: "merge owner", justification: "" },
    { accepter: "x", capacity: "merge owner", justification: " " },
    { accepter: "x", capacity: "", justification: "x" },
    { accepter: "x", capacity: " ", justification: "x" },
    { accepter: "x", capacity: "repository owner", justification: "x" },
  ];
  for (const disposition of ["Deferred", "Intentionally rejected"]) {
    for (const [index, acceptance] of acceptanceMutations.entries()) {
      rejects(
        `malformed ${disposition} acceptance ${index + 1} is refused`,
        () => validateResponses(majorLedger, [{
          ...acceptedMajor,
          disposition,
          acceptance,
        }]),
        /acceptance|capacity/,
      );
    }
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
  const documentedLateFinding = (overrides = {}) => ({
    severity: "high",
    introduced_regression: false,
    previously_missed: true,
    category: "correctness",
    source_id: "software:late-1",
    source_ordinal: 1,
    seat: "software",
    attribution: "software",
    raw_text: "An unsafe late finding.",
    description: "An unsafe late finding.",
    impact: "Approval is unsafe.",
    recommendation: "Fix it.",
    ...overrides,
  });
  check(
    "late-finding schema publishes the required admission fields",
    LATE_FINDING_SCHEMA.required.includes("introduced_regression") &&
      LATE_FINDING_SCHEMA.required.includes("previously_missed") &&
      LATE_FINDING_SCHEMA.required.includes("category") &&
      LATE_FINDING_SCHEMA.required.includes("seat") &&
      LATE_FINDING_SCHEMA.required.includes("raw_text") &&
      LATE_FINDING_SCHEMA.required.includes("recommendation"),
  );
  rejects(
    "late findings reject missing admission flags",
    () => lateFindingAdmission({
      ...documentedLateFinding(),
      introduced_regression: false,
      previously_missed: false,
    }),
    /introduced_regression or previously_missed/,
  );
  check(
    "late findings accept the documented exact schema",
    lateFindingAdmission(documentedLateFinding()).recommendation === "Fix it.",
  );
  rejects(
    "pre-existing late NIT is refused",
    () => lateFindingAdmission(documentedLateFinding({
      severity: "NIT",
      source_id: "software:late-2",
      raw_text: "A pre-existing style issue.",
      description: "A pre-existing style issue.",
      impact: "It is not a merge risk.",
      recommendation: "Record it without reopening discovery.",
    })),
    /not admissible/,
  );
  const admittedLateNit = lateFindingAdmission(documentedLateFinding({
      severity: "NIT",
      introduced_regression: true,
      previously_missed: false,
      source_id: "software:late-3",
      raw_text: "A new style regression.",
      description: "A new style regression.",
      impact: "The fix introduced a regression.",
      recommendation: "Correct the regression.",
    }));
  check(
    "introduced late NIT is admitted as a non-discovery regression",
    admittedLateNit.introduced_regression === true &&
      !Object.hasOwn(admittedLateNit, "late"),
  );
  const admittedLateMajor = lateFindingAdmission(documentedLateFinding({
      severity: "MAJOR",
      source_id: "software:late-4",
      raw_text: "A missed merge-risk issue.",
      description: "A missed merge-risk issue.",
      impact: "Approval would be unsafe.",
      recommendation: "Fix the issue.",
    }));
  check(
    "previously missed late MAJOR is admitted",
    admittedLateMajor.previously_missed === true &&
      Object.keys(admittedLateMajor).sort().join(",") ===
        [...LATE_FINDING_SCHEMA.required].sort().join(","),
  );
  const appendedFinding = documentedLateFinding({
    severity: "MAJOR",
    source_id: "software:late-5",
    raw_text: "A late unsafe issue.",
    description: "A late unsafe issue.",
    impact: "Approval would be unsafe.",
    recommendation: "Fix the issue.",
  });
  const appended = appendLateFindings(responseInput, [appendedFinding]);
  check("late issue receives the next stable R identifier", appended.issues.at(-1).id === "R5");
  rejects(
    "re-admitting the same late source is refused",
    () => appendLateFindings(appended, [appendedFinding]),
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
      content_id: "c".repeat(64),
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
        content_id: "c".repeat(64),
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
  const verificationIssueIds = responseInput.issues.map((issue) => issue.id);
  const canonicalVerificationVerdict = {
    engineer: "software",
    signoff: true,
    summary: "All ledger issues were verified.",
    verified_issue_statuses: verificationStatuses,
    late_findings: [],
    recommendations: [],
  };
  const actualVerificationVerdict = adaptVerificationVerdict(
    canonicalVerificationVerdict,
    { issue_ids: verificationIssueIds },
  );
  check(
    "actual verdict JSON adapts to explicit verification status",
    actualVerificationVerdict.verified_issue_statuses.R1 === "resolved" &&
      actualVerificationVerdict.signoff === true,
  );
  const nonEmptyAdaptedVerificationVerdict = adaptVerificationVerdict({
    ...canonicalVerificationVerdict,
    late_findings: [documentedLateFinding({
      source_id: "software:adapted-late",
    })],
  }, { issue_ids: verificationIssueIds });
  check(
    "non-empty verdict adaptation preserves the exact public late-finding schema",
    Object.keys(nonEmptyAdaptedVerificationVerdict.late_findings[0]).sort().join(",") ===
      [...LATE_FINDING_SCHEMA.required].sort().join(",") &&
      !Object.hasOwn(nonEmptyAdaptedVerificationVerdict.late_findings[0], "late"),
  );
  const verificationLedger = {
    ...responseInput,
    snapshot_sha256: "c".repeat(64),
  };
  const verificationArtifact = createVerificationResultArtifact({
    selection: verificationSelection.selection,
    selection_bytes: stableStringify(verificationSelection.selection),
    ledger: verificationLedger,
    ledger_bytes: stableStringify(verificationLedger),
    current_candidate: candidate({
      snapshot_sha256: "c".repeat(64),
      content_id: "c".repeat(64),
    }),
    results: allVerificationResults(
      verificationSelection.selection.roster,
      {},
      verificationLedger,
    ),
  });
  rejects(
    "verification rejects selection bytes that decode to another selection object",
    () => validateVerificationResultArtifact(verificationArtifact, {
      selection: verificationSelection.selection,
      selection_bytes: stableStringify({
        ...verificationSelection.selection,
        lifecycle_id: "foreign-lifecycle",
      }),
      ledger: verificationLedger,
      ledger_bytes: stableStringify(verificationLedger),
    }),
    /exact JSON bytes|staged artifact/,
  );
  rejects(
    "verification rejects a foreign lifecycle ledger",
    () => createVerificationResultArtifact({
      selection: verificationSelection.selection,
      selection_bytes: stableStringify(verificationSelection.selection),
      ledger: { ...verificationLedger, lifecycle_id: "foreign-lifecycle" },
      ledger_bytes: stableStringify({
        ...verificationLedger,
        lifecycle_id: "foreign-lifecycle",
      }),
      current_candidate: candidate({
        snapshot_sha256: "c".repeat(64),
        content_id: "c".repeat(64),
      }),
      results: allVerificationResults(
        verificationSelection.selection.roster,
        {},
        verificationLedger,
      ),
    }),
    /ledger and selection lifecycle_id disagree/,
  );
  const malformedCurrentVerificationVerdicts = [
    [
      "missing late_findings",
      (() => {
        const value = { ...canonicalVerificationVerdict };
        delete value.late_findings;
        return value;
      })(),
      /fields/,
    ],
    [
      "seat alias",
      (() => {
        const { engineer, ...value } = canonicalVerificationVerdict;
        return { ...value, seat: engineer };
      })(),
      /fields/,
    ],
    [
      "issue_statuses alias",
      (() => {
        const { verified_issue_statuses, ...value } = canonicalVerificationVerdict;
        return { ...value, issue_statuses: verified_issue_statuses };
      })(),
      /fields/,
    ],
    [
      "missing verified_issue_statuses default",
      (() => {
        const { verified_issue_statuses, ...value } = canonicalVerificationVerdict;
        return value;
      })(),
      /fields/,
    ],
    [
      "missing summary default",
      (() => {
        const { summary, ...value } = canonicalVerificationVerdict;
        return value;
      })(),
      /fields/,
    ],
    [
      "extra top-level field",
      { ...canonicalVerificationVerdict, complete: true },
      /fields/,
    ],
    [
      "non-array late_findings",
      { ...canonicalVerificationVerdict, late_findings: undefined },
      /late_findings/,
    ],
  ];
  for (const [name, malformed, pattern] of malformedCurrentVerificationVerdicts) {
    rejects(
      `malformed current verification verdict (${name}) is refused`,
      () => adaptVerificationVerdict(malformed, { issue_ids: verificationIssueIds }),
      pattern,
    );
  }
  rejects(
    "verification status aliases are refused without ledger issue ids",
    () => adaptVerificationVerdict({
      ...canonicalVerificationVerdict,
      verified_issue_statuses: [],
    }),
    /verified_issue_statuses/,
  );
  rejects(
    "a current verification recommendation must use the strict object shape",
    () => adaptVerificationVerdict({
      ...canonicalVerificationVerdict,
      signoff: false,
      recommendations: ["Fix the unresolved issue."],
    }, { issue_ids: verificationIssueIds }),
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
      verified_issue_statuses: verificationStatuses,
      late_findings: [],
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
    content_id: "c".repeat(64),
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
    /prior-ledger prefix|cover each issue|missing/,
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
                previously_missed: false,
                category: "correctness",
                source_id: "software:late-blocking",
                source_ordinal: 1,
                seat: "software",
                attribution: "software",
                raw_text: "A late blocking regression.",
                description: "A late blocking regression.",
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
  const introducedNitResults = Object.fromEntries(
    verificationSelection.selection.roster.map((seat) => [
      seat,
      {
        ...allVerificationResults(verificationSelection.selection.roster).software,
        seat,
        verified_issue_statuses: verificationStatuses,
        late_findings: seat === "software"
          ? [{
              severity: "NIT",
              introduced_regression: true,
              previously_missed: false,
              category: "correctness",
              source_id: "software:late-nit",
              source_ordinal: 1,
              seat: "software",
              attribution: "software",
              raw_text: "A newly introduced NIT regression.",
              description: "A newly introduced NIT regression.",
              impact: "The latest fix is not clean.",
              recommendation: "Correct the introduced regression.",
            }]
          : [],
      },
    ]),
  );
  const introducedNitApproval = evaluateApproval({
    selection: verificationSelection.selection,
    ledger: { ...responseInput, snapshot_sha256: "c".repeat(64) },
    responses,
    verification_results: introducedNitResults,
  });
  check(
    "introduced late NIT is blocking until continued",
    introducedNitApproval.late_blocking_issues.length === 1 &&
      introducedNitApproval.approved === false,
  );
  console.log("panel lifecycle: blocked verification advance");
  const advanceLedger = { ...responseInput };
  const advanceResponseEnvelope = createResponseTemplate(advanceLedger);
  advanceResponseEnvelope.responses = responses;
  const lateSpec003Finding = {
    severity: "MAJOR",
    introduced_regression: false,
    previously_missed: true,
    category: "correctness",
    source_id: "spec003:late-agentic",
    source_ordinal: 1,
    seat: "agentic",
    attribution: "agentic",
    raw_text: "Active Spec 003 instructions still require retired panels.",
    description: "Active Spec 003 instructions still require retired panels.",
    impact: "Operators can dispatch the wrong roster.",
    recommendation: "Replace fixed-count instructions with the selected roster.",
  };
  const advanceRawResults = allVerificationResults(
    verificationSelection.selection.roster,
    { agentic: [lateSpec003Finding] },
    advanceLedger,
  );
  advanceRawResults.agentic = adaptVerificationVerdict({
    engineer: "agentic",
    signoff: true,
    summary: "The late finding is recorded for continuation.",
    verified_issue_statuses: Object.fromEntries(
      advanceLedger.issues.map((issue) => [issue.id, "resolved"]),
    ),
    late_findings: [lateSpec003Finding],
    recommendations: [],
  }, {
    issue_ids: advanceLedger.issues.map((issue) => issue.id),
  });
  advanceRawResults.software.verified_issue_statuses.R2 = "open";
  const advanceVerificationArtifact = adaptedVerificationArtifact(
    verificationSelection.selection,
    advanceLedger,
    advanceRawResults,
  );
  const advanceInput = {
    current_selection: verificationSelection.selection,
    selection_bytes: stableStringify(verificationSelection.selection),
    discovery_ledger: advanceLedger,
    discovery_ledger_bytes: stableStringify(advanceLedger),
    responses: advanceResponseEnvelope,
    responses_bytes: stableStringify(advanceResponseEnvelope),
    verification_results: advanceVerificationArtifact,
    verification_results_bytes: stableStringify(advanceVerificationArtifact),
    current_candidate: candidate({
      snapshot_sha256: "c".repeat(64),
      content_id: "c".repeat(64),
    }),
  };
  const advanced = advanceVerification(advanceInput);
  check(
    "advance appends late findings with stable attribution and R identifiers",
    advanced.ledger.issues.at(-1).id === "R5" &&
      advanced.ledger.issues.at(-1).late === true &&
      advanced.ledger.sources.at(-1).source_id === "spec003:late-agentic" &&
      advanced.ledger.sources.at(-1).attribution === "agentic",
  );
  check(
    "advance rebinds the next ledger to the current candidate and roster",
    advanced.ledger.candidate_id === verificationSelection.selection.candidate_id &&
      advanced.ledger.snapshot_sha256 === verificationSelection.selection.snapshot_sha256 &&
      advanced.ledger.roster.join(",") === verificationSelection.selection.roster.join(","),
  );
  const advancedResponseById = new Map(
    advanced.responses.responses.map((response) => [response.issue_id, response]),
  );
  check(
    "advance carries passed responses and blanks every nonpassing or late issue",
    advancedResponseById.get("R1").disposition === "Fixed" &&
      advancedResponseById.get("R2").disposition === null &&
      advancedResponseById.get("R5").disposition === null &&
      advanced.reset_issue_ids.join(",") === "R2,R5",
  );
  const advanceOutput = join(root, "advance-handoff");
  const publishedAdvance = writeAdvanceVerification(advanceOutput, advanceInput);
  const repeatedAdvance = writeAdvanceVerification(advanceOutput, advanceInput);
  check(
    "advance publishes ledger and responses independently",
    publishedAdvance.publication.ledger.created === true &&
      publishedAdvance.publication.responses.created === true &&
      repeatedAdvance.publication.ledger.created === false &&
      repeatedAdvance.publication.responses.created === false &&
      readdirSync(advanceOutput).sort().join(",") ===
        "discovery-ledger.json,responses.json",
  );
  rmSync(join(advanceOutput, "responses.json"));
  const partialAdvance = writeAdvanceVerification(advanceOutput, advanceInput);
  check(
    "advance compares an existing ledger and creates a missing response",
    partialAdvance.publication.ledger.created === false &&
      partialAdvance.publication.responses.created === true,
  );
  const completedResponses = JSON.parse(stableStringify(advanced.responses));
  completedResponses.responses = completedResponses.responses.map((response) =>
    response.issue_id === "R2"
      ? {
          issue_id: "R2",
          disposition: "Fixed",
          changed_surface: ["src/panel.js"],
          justification: "The verification regression is fixed.",
          evidence: "focused lifecycle test",
        }
      : response.issue_id === "R5"
        ? {
            issue_id: "R5",
            disposition: "Fixed",
            changed_surface: ["specs/003-adr052-bazel-rust/plan.md"],
            justification: "The active instructions now use the selected roster.",
            evidence: "Spec 003 instruction audit",
          }
        : response
  );
  check(
    "a completed late response exposes the Spec 003 fix path to scope validation",
    validateFixScope({
      latest_delta_paths: ["specs/003-adr052-bazel-rust/plan.md"],
      responses: completedResponses.responses,
    }).latest_delta_paths[0] === "specs/003-adr052-bazel-rust/plan.md",
  );
  const nextCandidate = candidate({
    candidate_id: "e".repeat(64),
    content_id: "f".repeat(64),
    snapshot_sha256: "d".repeat(64),
    changed_paths: ["src/panel.js", "specs/003-adr052-bazel-rust/plan.md"],
  });
  const nextSelection = createSelection(
    {
      ...nextCandidate,
      lifecycle_id: "spec004w1",
      phase: "verification",
      full_candidate: nextCandidate,
      fix_delta: { changed_paths: ["specs/003-adr052-bazel-rust/plan.md"] },
      previous_selection: verificationSelection.selection,
    },
    { root },
  );
  const nextPriorVerdicts = Object.fromEntries(
    verificationSelection.selection.roster.map((seat) => [
      seat,
      {
        engineer: seat,
        signoff: false,
        summary: "The blocked verification is being continued.",
        verified_issue_statuses: Object.fromEntries(
          advanced.ledger.issues
            .filter((issue) => issue.late !== true)
            .map((issue) => [issue.id, "verified"]),
        ),
        late_findings: [],
        recommendations: [{
          severity: "high",
          where: "panel",
          what: "A verification issue remains.",
          why: "The next fix must be reviewed.",
          fix: "Verify the next response.",
        }],
      },
    ]),
  );
  rejects(
    "verification prior verdicts use the prior selection phase schema",
    () => prepareVerification({
      selection: nextSelection.selection,
      ledger: advanced.ledger,
      responses: completedResponses,
      self_verification: selfVerification,
      current_candidate: nextCandidate,
      prior_selection: verificationSelection.selection,
      prior_verdicts: {
        ...nextPriorVerdicts,
        software: {
          engineer: "software",
          signoff: true,
          summary: "Wrong discovery-shaped prior result.",
          recommendations: [],
        },
      },
      latest_delta_paths: ["specs/003-adr052-bazel-rust/plan.md"],
    }),
    /fields.*verified_issue_statuses|exactly/,
  );
  rejects(
    "verification prior verdicts require complete status coverage",
    () => prepareVerification({
      selection: nextSelection.selection,
      ledger: advanced.ledger,
      responses: completedResponses,
      self_verification: selfVerification,
      current_candidate: nextCandidate,
      prior_selection: verificationSelection.selection,
      prior_verdicts: {
        ...nextPriorVerdicts,
        software: {
          ...nextPriorVerdicts.software,
          verified_issue_statuses: { R1: "verified" },
        },
      },
      latest_delta_paths: ["specs/003-adr052-bazel-rust/plan.md"],
    }),
    /prior-ledger prefix|cover each issue|missing/,
  );
  const nextPreparation = prepareVerification({
    selection: nextSelection.selection,
    ledger: advanced.ledger,
    responses: completedResponses,
    self_verification: selfVerification,
    current_candidate: nextCandidate,
    prior_selection: verificationSelection.selection,
    prior_verdicts: nextPriorVerdicts,
    latest_delta_paths: ["specs/003-adr052-bazel-rust/plan.md"],
  });
  check(
    "advance output feeds rerun selection and verification preparation",
    nextPreparation.scope.latest_delta_paths.join(",") ===
      "specs/003-adr052-bazel-rust/plan.md",
  );
  const secondLateFinding = {
    ...lateSpec003Finding,
    source_id: "spec003:late-build",
    source_ordinal: 2,
    raw_text: "A second late issue remains in the continued ledger.",
    description: "A second late issue remains in the continued ledger.",
    recommendation: "Resolve the second late issue.",
  };
  const prefixLedger = appendLateFindings(advanced.ledger, [secondLateFinding]);
  const prefixResponses = createResponseTemplate(prefixLedger);
  prefixResponses.responses = prefixResponses.responses.map((response) => ({
    ...response,
    disposition: "Fixed",
    changed_surface: ["specs/003-adr052-bazel-rust/plan.md"],
    justification: "The continued issue is fixed.",
    evidence: "focused continuation test",
  }));
  const priorVerificationWithIntermediatePrefix = Object.fromEntries(
    verificationSelection.selection.roster.map((seat) => [
      seat,
      {
        engineer: seat,
        signoff: true,
        summary: "The prior ledger prefix was verified.",
        verified_issue_statuses: Object.fromEntries(
          prefixLedger.issues.slice(0, 5).map((issue) => [issue.id, "verified"]),
        ),
        late_findings: [],
        recommendations: [],
      },
    ]),
  );
  check(
    "prior verification accepts an intermediate contiguous prefix including a late issue",
    prepareVerification({
      selection: nextSelection.selection,
      ledger: prefixLedger,
      responses: prefixResponses,
      self_verification: selfVerification,
      current_candidate: nextCandidate,
      prior_selection: verificationSelection.selection,
      prior_verdicts: priorVerificationWithIntermediatePrefix,
      latest_delta_paths: ["specs/003-adr052-bazel-rust/plan.md"],
    }).requests.length === nextSelection.selection.roster.length,
  );
  const nonContiguousPrefixStatuses = Object.fromEntries(
    prefixLedger.issues
      .slice(0, 5)
      .filter((_, index) => index !== 2)
      .map((issue) => [issue.id, "verified"]),
  );
  rejects(
    "prior verification rejects a non-contiguous same-sized prefix",
    () => prepareVerification({
      selection: nextSelection.selection,
      ledger: prefixLedger,
      responses: prefixResponses,
      self_verification: selfVerification,
      current_candidate: nextCandidate,
      prior_selection: verificationSelection.selection,
      prior_verdicts: Object.fromEntries(
        verificationSelection.selection.roster.map((seat) => [
          seat,
          {
            ...priorVerificationWithIntermediatePrefix[seat],
            verified_issue_statuses: nonContiguousPrefixStatuses,
          },
        ]),
      ),
      latest_delta_paths: ["specs/003-adr052-bazel-rust/plan.md"],
    }),
    /prior-ledger prefix|cover each issue|missing|extra/,
  );
  rejects(
    "advance rejects a duplicate late source identity",
    () => {
      const duplicateResults = JSON.parse(stableStringify(advanceRawResults));
      duplicateResults.software.late_findings = [lateSpec003Finding];
      const duplicateArtifact = adaptedVerificationArtifact(
        verificationSelection.selection,
        advanceLedger,
        duplicateResults,
      );
      advanceVerification({
        ...advanceInput,
        verification_results: duplicateArtifact,
        verification_results_bytes: stableStringify(duplicateArtifact),
      });
    },
    /late source finding.*already exists/,
  );
  rejects(
    "advance rejects a missing prior response",
    () => {
      const missing = JSON.parse(stableStringify(advanceResponseEnvelope));
      missing.responses = missing.responses.slice(0, -1);
      advanceVerification({
        ...advanceInput,
        responses: missing,
        responses_bytes: stableStringify(missing),
      });
    },
    /missing implementation responses/,
  );
  rejects(
    "advance rejects a selection digest mismatch",
    () => advanceVerification({
      ...advanceInput,
      selection_bytes: `${stableStringify(verificationSelection.selection)}\n`,
    }),
    /selection bytes|exact selection bytes/,
  );
  rejects(
    "advance rejects a candidate and selection mismatch",
    () => advanceVerification({
      ...advanceInput,
      current_candidate: candidate({
        candidate_id: "different-candidate",
        snapshot_sha256: "c".repeat(64),
        content_id: "c".repeat(64),
      }),
    }),
    /selection candidate mismatch|candidate_id/,
  );
  rejects(
    "advance rejects malformed blocked status input",
    () => {
      const malformedResults = JSON.parse(
        stableStringify(advanceVerificationArtifact),
      );
      Object.values(malformedResults.results).find((result) => result.seat === "software")
        .verified_issue_statuses.R1 = { status: "blocked", extra: "no" };
      advanceVerification({
        ...advanceInput,
        verification_results: malformedResults,
        verification_results_bytes: stableStringify(malformedResults),
      });
    },
    /status|only/,
  );
  const verificationDir = join(root, "verification");
  const writtenVerification = writeVerificationArtifacts(verificationDir, {
    selection: verificationSelection.selection,
    ledger: { ...responseInput, snapshot_sha256: "c".repeat(64) },
    responses,
    self_verification: selfVerification,
    current_candidate: candidate({
      snapshot_sha256: "c".repeat(64),
      content_id: "c".repeat(64),
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
        content_id: "c".repeat(64),
      }),
      prior_selection: initial.selection,
      prior_verdicts: priorVerdicts,
      latest_delta_paths: ["src/panel.js"],
    }),
    /incomplete or has extra entries/,
  );
  const identicalFamily = join(root, "concurrent-identical-family");
  const identicalResults = await concurrentDirectoryPublish(
    identicalFamily,
    LIFECYCLE_CLI,
    ["identical\n", "identical\n"],
  );
  check(
    "concurrent identical directory publishers compare the winning family",
    identicalResults.filter((result) => result.status === 0).length === 2 &&
      identicalResults.filter((result) => /"created":true/.test(result.stdout)).length === 1 &&
      readFileSync(join(identicalFamily, "seat.json"), "utf8") === "identical\n",
    identicalResults.map((result) => `${result.stdout}${result.stderr}`).join(" "),
  );
  check(
    "an identical directory family is accepted without replacement",
    writeDirectoryCreateOrCompare(identicalFamily, [
      { name: "seat.json", bytes: "identical\n" },
    ]).created === false,
  );
  rejects(
    "a conflicting directory family is refused",
    () => writeDirectoryCreateOrCompare(identicalFamily, [
      { name: "seat.json", bytes: "different\n" },
    ]),
    /conflicting generated bytes/,
  );
  const conflictingFamily = join(root, "concurrent-conflicting-family");
  const conflictingResults = await concurrentDirectoryPublish(
    conflictingFamily,
    LIFECYCLE_CLI,
    ["first\n", "second\n"],
  );
  check(
    "concurrent conflicting directory publishers reject the loser",
    conflictingResults.filter((result) => result.status === 0).length === 1 &&
      conflictingResults.filter((result) => result.status !== 0).length === 1 &&
      conflictingResults.filter((result) => /"created":true/.test(result.stdout)).length === 1 &&
      ["first\n", "second\n"].includes(
        readFileSync(join(conflictingFamily, "seat.json"), "utf8"),
      ) &&
      conflictingResults.some((result) => /conflicting generated bytes/.test(result.stderr)),
    conflictingResults.map((result) => `${result.stdout}${result.stderr}`).join(" "),
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
    cpSync(
      join(fileURLToPath(new URL(".", import.meta.url)), "..", "..", ".github", "skills", "d2b-panel-round", "dispatch-policy.json"),
      join(cliRoot, ".github", "skills", "d2b-panel-round", "dispatch-policy.json"),
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
    const address = (snapshot, content = "2".repeat(64)) => ({
      program: "SPEC004",
      wave: "spec004w1",
      candidate_id: "1".repeat(64),
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
    const discoveryVerdictObjects = Object.fromEntries(
      discoveryRoster.map((seat) => [seat, {
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
      }]),
    );
    for (const seat of discoveryRoster) {
      writeFileSync(
        join(cliFirstRound, "verdicts", `${seat}.json`),
        stableStringify(discoveryVerdictObjects[seat]),
      );
    }
    runCli(
      "adapt-discovery",
      join(cliFirstRound, "verdicts"),
      cliDiscoveryResults,
      "--selection",
      join(cliFirstRound, "selection.json"),
      "--candidate",
      join(cliFirstRound, "current-candidate.json"),
    );
    const cliAdaptedDiscovery = JSON.parse(
      readFileSync(cliDiscoveryResults, "utf8"),
    );
    check(
      "adapt-discovery binds the staged selection, candidate, lifecycle, and roster order",
      cliAdaptedDiscovery.lifecycle_id === "spec004w1" &&
        cliAdaptedDiscovery.selection_sha256 === sha256(
          readFileSync(join(cliFirstRound, "selection.json"), "utf8"),
        ) &&
        cliAdaptedDiscovery.current_candidate.snapshot_sha256 ===
          "d".repeat(64) &&
        cliAdaptedDiscovery.results.map((result) => result.seat).join(",") ===
          discoveryRoster.join(","),
    );
    const adaptDiscoveryDirectory = (directory, output) => spawnSync("node", [
      LIFECYCLE_CLI,
      "adapt-discovery",
      directory,
      output,
      "--selection",
      join(cliFirstRound, "selection.json"),
      "--candidate",
      join(cliFirstRound, "current-candidate.json"),
    ], { cwd: cliRoot, encoding: "utf8" });
    const incompleteDiscoveryVerdicts = join(cliRoot, "incomplete-discovery-verdicts");
    cpSync(join(cliFirstRound, "verdicts"), incompleteDiscoveryVerdicts, {
      recursive: true,
    });
    rmSync(join(incompleteDiscoveryVerdicts, `${discoveryRoster.at(-1)}.json`));
    const incompleteDiscovery = adaptDiscoveryDirectory(
      incompleteDiscoveryVerdicts,
      join(cliRoot, "incomplete-discovery-results.json"),
    );
    check(
      "adapt-discovery refuses an incomplete selected-seat directory",
      incompleteDiscovery.status !== 0 &&
        /missing selected seat/.test(
          `${incompleteDiscovery.stdout}${incompleteDiscovery.stderr}`,
        ),
    );
    const unselectedSeat = readSelectionTable().optional_seats.find(
      (seat) => !discoveryRoster.includes(seat),
    );
    const unselectedDiscoveryVerdicts = join(cliRoot, "unselected-discovery-verdicts");
    cpSync(join(cliFirstRound, "verdicts"), unselectedDiscoveryVerdicts, {
      recursive: true,
    });
    writeFileSync(
      join(unselectedDiscoveryVerdicts, `${unselectedSeat}.json`),
      stableStringify({
        engineer: unselectedSeat,
        signoff: true,
        summary: "No discovery findings.",
        recommendations: [],
      }),
    );
    const unselectedDiscovery = adaptDiscoveryDirectory(
      unselectedDiscoveryVerdicts,
      join(cliRoot, "unselected-discovery-results.json"),
    );
    check(
      "adapt-discovery refuses an unselected verdict filename",
      unselectedDiscovery.status !== 0 &&
        /unselected seat|more than .* entries/.test(
          `${unselectedDiscovery.stdout}${unselectedDiscovery.stderr}`,
        ),
    );
    const mismatchedDiscoveryVerdicts = join(cliRoot, "mismatched-discovery-verdicts");
    cpSync(join(cliFirstRound, "verdicts"), mismatchedDiscoveryVerdicts, {
      recursive: true,
    });
    const mismatchedSeat = discoveryRoster.at(-1);
    writeFileSync(
      join(mismatchedDiscoveryVerdicts, `${mismatchedSeat}.json`),
      stableStringify({
        ...discoveryVerdictObjects[mismatchedSeat],
        engineer: unselectedSeat,
      }),
    );
    const mismatchedDiscovery = adaptDiscoveryDirectory(
      mismatchedDiscoveryVerdicts,
      join(cliRoot, "mismatched-discovery-results.json"),
    );
    check(
      "adapt-discovery refuses a filename and declared-seat mismatch",
      mismatchedDiscovery.status !== 0 &&
        /filename and selected seat must agree/.test(
          `${mismatchedDiscovery.stdout}${mismatchedDiscovery.stderr}`,
        ),
    );
    const duplicateDiscoveryDirectory = join(cliRoot, "duplicate-discovery-verdicts");
    cpSync(join(cliFirstRound, "verdicts"), duplicateDiscoveryDirectory, {
      recursive: true,
    });
    writeFileSync(
      join(duplicateDiscoveryDirectory, `${discoveryRoster[1]}.json`),
      stableStringify({
        ...discoveryVerdictObjects[discoveryRoster[1]],
        engineer: discoveryRoster[0],
      }),
    );
    const duplicateDiscovery = adaptDiscoveryDirectory(
      duplicateDiscoveryDirectory,
      join(cliRoot, "duplicate-discovery-results.json"),
    );
    check(
      "adapt-discovery refuses duplicate declared seats",
      duplicateDiscovery.status !== 0 &&
        /duplicate declared seat/.test(
          `${duplicateDiscovery.stdout}${duplicateDiscovery.stderr}`,
        ),
    );
    const malformedDiscoveryDirectory = join(
      cliRoot,
      "malformed-discovery-verdicts",
    );
    cpSync(join(cliFirstRound, "verdicts"), malformedDiscoveryDirectory, {
      recursive: true,
    });
    const malformedDiscoveryFilename = `${discoveryRoster[0]}.json`;
    writeFileSync(
      join(malformedDiscoveryDirectory, malformedDiscoveryFilename),
      "{ malformed\n",
    );
    const malformedDiscovery = adaptDiscoveryDirectory(
      malformedDiscoveryDirectory,
      join(cliRoot, "malformed-discovery-results.json"),
    );
    const malformedDiscoveryText =
      `${malformedDiscovery.stdout}${malformedDiscovery.stderr}`;
    check(
      "adapt-discovery malformed JSON names its directory and entry filename",
      malformedDiscovery.status !== 0 &&
        malformedDiscoveryText.includes(malformedDiscoveryDirectory) &&
        malformedDiscoveryText.includes(malformedDiscoveryFilename),
      malformedDiscoveryText,
    );
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
    runCli(
      "merge-ledger",
      discoverySelection,
      cliDiscoveryResults,
      cliGroups,
      cliLedger,
      "--candidate",
      join(cliFirstRound, "current-candidate.json"),
    );
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
      stableStringify(address("e".repeat(64), "3".repeat(64))),
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
        verified_issue_statuses: Object.fromEntries(ledgerIssues.map((issue) => [issue.id, "resolved"])),
        late_findings: [],
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
      "approval CLI exits 0 and exposes an approved artifact",
      approvalResult.status === 0 &&
        JSON.parse(readFileSync(cliApproval, "utf8")).approved === true,
      `${approvalResult.stdout}${approvalResult.stderr}`,
    );
    const blockedVerificationResultsPath = join(
      cliRoot,
      "blocked-verification-results.json",
    );
    const blockedVerificationResults = JSON.parse(
      readFileSync(cliVerificationResults, "utf8"),
    );
    blockedVerificationResults.results[0].signoff = false;
    blockedVerificationResults.results[0].verified_issue_statuses.R1 = "open";
    blockedVerificationResults.results[0].recommendations = [{
      severity: "high",
      where: "src/panel.js:1",
      what: "The source mapping remains open.",
      why: "The ledger issue is not resolved.",
      fix: "Complete and verify the source mapping fix.",
    }];
    blockedVerificationResults.results[0].late_findings = [{
      severity: "high",
      introduced_regression: false,
      previously_missed: true,
      category: "correctness",
      source_id: "spec003:cli-late",
      source_ordinal: 1,
      seat: verificationRoster[0],
      attribution: verificationRoster[0],
      raw_text: "Spec 003 still names the retired panel roster.",
      description: "Spec 003 still names the retired panel roster.",
      impact: "The next fix path must be admitted to the ledger.",
      recommendation: "Use the selected roster in the active instructions.",
    }];
    writeFileSync(
      blockedVerificationResultsPath,
      stableStringify(blockedVerificationResults),
    );
    const blockedApprovalPath = join(cliRoot, "blocked-approval.json");
    const blockedApprovalResult = spawnSync("node", [
      LIFECYCLE_CLI,
      "approval",
      join(cliRound, "selection.json"),
      join(cliRound, "discovery-ledger.json"),
      join(cliRound, "responses.json"),
      blockedVerificationResultsPath,
      blockedApprovalPath,
      "--candidate",
      join(cliRound, "current-candidate.json"),
    ], { cwd: cliRoot, encoding: "utf8" });
    check(
      "approval CLI exits 3 and writes a valid blocked artifact",
      blockedApprovalResult.status === 3 &&
        JSON.parse(readFileSync(blockedApprovalPath, "utf8")).approved === false,
      `${blockedApprovalResult.stdout}${blockedApprovalResult.stderr}`,
    );
    const cliAdvanceDirectory = join(cliRoot, "advance-handoff");
    const advanceVerificationResult = spawnSync("node", [
      LIFECYCLE_CLI,
      "advance-verification",
      join(cliRound, "selection.json"),
      join(cliRound, "discovery-ledger.json"),
      join(cliRound, "responses.json"),
      blockedVerificationResultsPath,
      cliAdvanceDirectory,
      "--candidate",
      join(cliRound, "current-candidate.json"),
    ], { cwd: cliRoot, encoding: "utf8" });
    const advancedCliLedger = JSON.parse(
      readFileSync(join(cliAdvanceDirectory, "discovery-ledger.json"), "utf8"),
    );
    const advancedCliResponses = JSON.parse(
      readFileSync(join(cliAdvanceDirectory, "responses.json"), "utf8"),
    );
    check(
      "advance-verification CLI publishes the blocked handoff",
      advanceVerificationResult.status === 0 &&
        advancedCliLedger.issues.at(-1).id === "R2" &&
        advancedCliLedger.sources.at(-1).source_id === "spec003:cli-late" &&
        advancedCliResponses.responses.find((response) => response.issue_id === "R1")
          .disposition === null &&
        readdirSync(cliAdvanceDirectory).sort().join(",") ===
          "discovery-ledger.json,responses.json",
      `${advanceVerificationResult.stdout}${advanceVerificationResult.stderr}`,
    );
    const repeatedAdvanceVerification = spawnSync("node", [
      LIFECYCLE_CLI,
      "advance-verification",
      join(cliRound, "selection.json"),
      join(cliRound, "discovery-ledger.json"),
      join(cliRound, "responses.json"),
      blockedVerificationResultsPath,
      cliAdvanceDirectory,
      "--candidate",
      join(cliRound, "current-candidate.json"),
    ], { cwd: cliRoot, encoding: "utf8" });
    check(
      "advance-verification CLI compares an identical existing handoff",
      repeatedAdvanceVerification.status === 0,
      `${repeatedAdvanceVerification.stdout}${repeatedAdvanceVerification.stderr}`,
    );
    const unsupportedAdvanceLifecycle = spawnSync("node", [
      LIFECYCLE_CLI,
      "advance-verification",
      join(cliRound, "selection.json"),
      join(cliRound, "discovery-ledger.json"),
      join(cliRound, "responses.json"),
      blockedVerificationResultsPath,
      join(cliRoot, "unsupported-lifecycle-handoff"),
      "--candidate",
      join(cliRound, "current-candidate.json"),
      "--lifecycle",
      "spec004w1",
    ], { cwd: cliRoot, encoding: "utf8" });
    check(
      "advance-verification CLI rejects its unsupported lifecycle flag",
      unsupportedAdvanceLifecycle.status !== 0 &&
        /does not recognize argument/.test(
          `${unsupportedAdvanceLifecycle.stdout}${unsupportedAdvanceLifecycle.stderr}`,
        ),
      `${unsupportedAdvanceLifecycle.stdout}${unsupportedAdvanceLifecycle.stderr}`,
    );
    const conflictingAdvanceResultsPath = join(
      cliRoot,
      "conflicting-blocked-verification-results.json",
    );
    blockedVerificationResults.results[0].late_findings[0].recommendation =
      "Use the exact selected roster and profile in active instructions.";
    writeFileSync(
      conflictingAdvanceResultsPath,
      stableStringify(blockedVerificationResults),
    );
    const conflictingAdvanceVerification = spawnSync("node", [
      LIFECYCLE_CLI,
      "advance-verification",
      join(cliRound, "selection.json"),
      join(cliRound, "discovery-ledger.json"),
      join(cliRound, "responses.json"),
      conflictingAdvanceResultsPath,
      cliAdvanceDirectory,
      "--candidate",
      join(cliRound, "current-candidate.json"),
    ], { cwd: cliRoot, encoding: "utf8" });
    check(
      "advance-verification CLI refuses conflicting regeneration",
      conflictingAdvanceVerification.status !== 0 &&
        /conflicting generated bytes/.test(
          `${conflictingAdvanceVerification.stdout}${conflictingAdvanceVerification.stderr}`,
        ),
      `${conflictingAdvanceVerification.stdout}${conflictingAdvanceVerification.stderr}`,
    );
    const invalidApprovalInvocation = spawnSync("node", [
      LIFECYCLE_CLI,
      "approval",
      join(cliRound, "selection.json"),
      join(cliRound, "discovery-ledger.json"),
      join(cliRound, "responses.json"),
      join(cliRound, "verification-results.json"),
      join(cliRoot, "invalid-invocation-approval.json"),
    ], { cwd: cliRoot, encoding: "utf8" });
    check(
      "approval CLI exits 2 for an invalid invocation",
      invalidApprovalInvocation.status === 2 &&
        /approval requires --candidate/.test(
          `${invalidApprovalInvocation.stdout}${invalidApprovalInvocation.stderr}`,
        ),
    );
    const unknownApprovalFlag = spawnSync("node", [
      LIFECYCLE_CLI,
      "approval",
      join(cliRound, "selection.json"),
      join(cliRound, "discovery-ledger.json"),
      join(cliRound, "responses.json"),
      join(cliRound, "verification-results.json"),
      join(cliRoot, "unknown-flag-approval.json"),
      "--candidate",
      join(cliRound, "current-candidate.json"),
      "--unknown",
      "value",
    ], { cwd: cliRoot, encoding: "utf8" });
    check(
      "approval CLI refuses unknown flags",
      unknownApprovalFlag.status === 2 &&
        /does not recognize argument/.test(
          `${unknownApprovalFlag.stdout}${unknownApprovalFlag.stderr}`,
        ),
    );
    const duplicateApprovalFlag = spawnSync("node", [
      LIFECYCLE_CLI,
      "approval",
      join(cliRound, "selection.json"),
      join(cliRound, "discovery-ledger.json"),
      join(cliRound, "responses.json"),
      join(cliRound, "verification-results.json"),
      join(cliRoot, "duplicate-flag-approval.json"),
      "--candidate",
      join(cliRound, "current-candidate.json"),
      "--candidate",
      join(cliRound, "current-candidate.json"),
    ], { cwd: cliRoot, encoding: "utf8" });
    check(
      "approval CLI refuses duplicate flags",
      duplicateApprovalFlag.status === 2 &&
        /duplicate flag/.test(
          `${duplicateApprovalFlag.stdout}${duplicateApprovalFlag.stderr}`,
        ),
    );
    const surplusApprovalPositional = spawnSync("node", [
      LIFECYCLE_CLI,
      "approval",
      join(cliRound, "selection.json"),
      join(cliRound, "discovery-ledger.json"),
      join(cliRound, "responses.json"),
      join(cliRound, "verification-results.json"),
      join(cliRoot, "surplus-positional-approval.json"),
      "unexpected",
      "--candidate",
      join(cliRound, "current-candidate.json"),
    ], { cwd: cliRoot, encoding: "utf8" });
    check(
      "approval CLI refuses surplus positional arguments",
      surplusApprovalPositional.status === 2 &&
        /exactly 5 positional/.test(
          `${surplusApprovalPositional.stdout}${surplusApprovalPositional.stderr}`,
        ),
    );
    const invalidApprovalResponses = join(cliRoot, "invalid-approval-responses.json");
    writeFileSync(invalidApprovalResponses, "{\n");
    const invalidApprovalInput = spawnSync("node", [
      LIFECYCLE_CLI,
      "approval",
      join(cliRound, "selection.json"),
      join(cliRound, "discovery-ledger.json"),
      invalidApprovalResponses,
      join(cliRound, "verification-results.json"),
      join(cliRoot, "invalid-input-approval.json"),
      "--candidate",
      join(cliRound, "current-candidate.json"),
    ], { cwd: cliRoot, encoding: "utf8" });
    check(
      "approval CLI exits 2 for invalid input",
      invalidApprovalInput.status === 2,
      `${invalidApprovalInput.stdout}${invalidApprovalInput.stderr}`,
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
      Object.fromEntries(verificationRoster.map((seat, index) => {
        const definition = readFileSync(
          join(cliRoot, ".github", "agents", `panel-${seat}.agent.md`),
        );
        return [seat, {
          provider: "github-copilot",
          model: "gpt-5.6-sol",
          reasoning_effort: "xhigh",
          context_tier: "default",
          communication: "caveman-full-optional",
          agent_type: `panel-${seat}`,
          agent_definition_sha256: sha256(definition.toString("utf8")),
          run_id: `cli-run-${index}`,
          receipt_locator: `github-copilot://cli/${index}`,
        }];
      })),
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
  const shuffledLegacyAttestation = {
    ...exactLegacyAttestation,
    records: [
      exactLegacyAttestation.records[1],
      exactLegacyAttestation.records[0],
      ...exactLegacyAttestation.records.slice(2),
    ],
  };
  rejects(
    "legacy attestation rejects records outside historical roster order",
    () => importLegacyRound({
      request: legacyRequest,
      records: legacyWireRecords,
      record_bytes: exactLegacyRecordBytes,
      attestation: shuffledLegacyAttestation,
    }, { candidate: legacyCandidate }),
    /roster order|expected software/,
  );
  const shuffledLegacySealPanel = {
    ...exactLegacyAttestation,
    records: [
      exactLegacyAttestation.records[1],
      exactLegacyAttestation.records[0],
      ...exactLegacyAttestation.records.slice(2),
    ],
  };
  rejects(
    "legacy seal-panel rejects records outside historical roster order",
    () => importLegacyRound({
      request: legacyRequest,
      records: legacyWireRecords,
      record_bytes: exactLegacyRecordBytes,
      seal_panel: shuffledLegacySealPanel,
    }, { candidate: legacyCandidate }),
    /roster order|expected software/,
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
  const continuedComplete = continueLegacyImport(exactObjectImport, {
    selection: priorSelection,
    candidate: candidate(),
  });
  check(
    "complete legacy imports continue directly into an issue ledger",
    continuedComplete.discovery_required === false &&
      continuedComplete.artifact.artifact_kind === "d2b-panel/issue-ledger" &&
      continuedComplete.artifact.issues.length === legacyWireRecords.length,
  );
  const continuedPartial = continueLegacyImport(exactPartialImport, {
    selection: priorSelection,
    candidate: candidate(),
  });
  check(
    "partial legacy imports continue into one current discovery request",
    continuedPartial.discovery_required === true &&
      continuedPartial.artifact.artifact_kind === "d2b-panel/discovery-request" &&
      continuedPartial.artifact.context.legacy_import.sources.length === 2,
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
    const legacySoftwarePath = join(
      legacyDir,
      "records",
      "software.json",
    );
    writeFileSync(legacySoftwarePath, "{ malformed\n");
    try {
      importLegacyRound(legacyDir, { candidate: legacyCandidate });
      check("legacy record parse failures name directory and filename", false);
    } catch (cause) {
      check(
        "legacy record parse failures name directory and filename",
        cause.message.includes(join(legacyDir, "records")) &&
          cause.message.includes("software.json") &&
          /malformed JSON/.test(cause.message),
        cause.message,
      );
    }
    writeFileSync(legacySoftwarePath, stableStringify(legacyWireRecords[0]));
    writeFileSync(
      join(legacyDir, "records", "unexpected.json"),
      "{ malformed\n",
    );
    rejects(
      "legacy directory rejects non-fixed-ten names before parsing records",
      () => importLegacyRound(legacyDir, { candidate: legacyCandidate }),
      /incomplete or has extra entries|more than 10 entries/,
    );
    rmSync(join(legacyDir, "records", "unexpected.json"));
    writeFileSync(
      legacySoftwarePath,
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
  const partialLegacyDir = mkdtempSync(join(tmpdir(), "d2b-legacy-partial-round-"));
  try {
    mkdirSync(join(partialLegacyDir, "records"));
    writeFileSync(
      join(partialLegacyDir, "panel-request.json"),
      stableStringify(legacyRequest),
    );
    for (const record of legacyWireRecords.slice(0, 2)) {
      writeFileSync(
        join(partialLegacyDir, "records", `${record.role}.json`),
        stableStringify(record),
      );
    }
    const partialDirectoryImport = importLegacyRound(partialLegacyDir, {
      candidate: legacyCandidate,
    });
    check(
      "legacy directory import accepts a validated partial record subset",
      partialDirectoryImport.complete === false &&
        partialDirectoryImport.sources.length === 2 &&
        partialDirectoryImport.discovery_required === true,
    );
  } finally {
    rmSync(partialLegacyDir, { recursive: true, force: true });
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
