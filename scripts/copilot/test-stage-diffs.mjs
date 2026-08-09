#!/usr/bin/env node
// Coverage for the staged panel review request. The integrator dispatches the
// generated prompt verbatim, so this test proves that the prompt carries the
// incremental range, full context, evidence, prior-verdict obligation, and
// no-rerun rule instead of relying on a hand-written task prompt.

import {
  chmodSync,
  cpSync,
  existsSync,
  linkSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { createHash } from "node:crypto";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { execFileSync, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const here = fileURLToPath(new URL(".", import.meta.url));
const root = join(here, "..", "..");
const script = join(
  root,
  ".github",
  "skills",
  "d2b-panel-round",
  "scripts",
  "stage-diffs.sh",
);
const lifecycleScript = join(
  root,
  ".github",
  "skills",
  "d2b-panel-round",
  "scripts",
  "panel-lifecycle.mjs",
);
const selectionTable = join(
  root,
  ".github",
  "skills",
  "d2b-panel-round",
  "selection-table.json",
);

let failures = 0;
const check = (name, ok, detail = "") => {
  if (ok) {
    console.log(`  ok   ${name}`);
    return;
  }
  failures += 1;
  console.error(`  FAIL ${name}${detail ? `: ${detail}` : ""}`);
};

let finalizedEvidencePath = "";

function run(cwd, args, options = {}) {
  const withEvidence =
    options.includeEvidence === false ||
    !finalizedEvidencePath ||
    args.includes("--evidence")
      ? args
      : [...args, "--evidence", finalizedEvidencePath];
  const result = spawnSync("bash", [script, ...withEvidence], {
    cwd,
    encoding: "utf8",
    env: {
      ...process.env,
      ...(options.env ?? {}),
    },
  });
  return {
    status: result.status,
    text: `${result.stdout || ""}${result.stderr || ""}`,
  };
}

function git(cwd, ...args) {
  const result = spawnSync("git", args, { cwd, encoding: "utf8" });
  if (result.status !== 0) {
    throw new Error(
      `git ${args.join(" ")} failed: ${result.stderr || result.stdout}`,
    );
  }
  return result.stdout.trim();
}

const repo = mkdtempSync(join(tmpdir(), "d2b-stage-diffs-"));
try {
  git(repo, "init", "--quiet");
  git(repo, "config", "user.name", "d2b test");
  git(repo, "config", "user.email", "d2b-test@example.invalid");

  const agents = join(repo, ".github", "agents");
  mkdirSync(agents, { recursive: true });
  const skillScripts = join(
    repo,
    ".github",
    "skills",
    "d2b-panel-round",
    "scripts",
  );
  mkdirSync(skillScripts, { recursive: true });
  cpSync(lifecycleScript, join(skillScripts, "panel-lifecycle.mjs"));
  cpSync(selectionTable, join(repo, ".github", "skills", "d2b-panel-round", "selection-table.json"));
  for (const seat of [
    "software", "test", "product", "docs", "security", "observability",
    "simplicity", "reliability", "agentic", "nixos", "build",
  ]) {
    writeFileSync(join(agents, `panel-${seat}.agent.md`), `name: ${seat}\n`);
  }

  writeFileSync(join(repo, "base.txt"), "base\n");
  git(repo, "add", "base.txt", ".github/agents");
  git(repo, "commit", "--quiet", "-m", "base");
  const base = git(repo, "rev-parse", "HEAD");

  const literalBackslashPath = "literal\\backslash.txt";
  writeFileSync(join(repo, "first.txt"), "first change\n");
  writeFileSync(join(repo, literalBackslashPath), "literal backslash change\n");
  git(repo, "add", "first.txt", literalBackslashPath);
  git(repo, "commit", "--quiet", "-m", "first");
  const firstTip = git(repo, "rev-parse", "HEAD");

  finalizedEvidencePath = join(repo, "finalized-evidence.md");
  writeFileSync(
    finalizedEvidencePath,
    "# Validation evidence\n\n| Command | Result |\n|---|---|\n| focused | PASS |\n",
  );
  const finalizedEvidenceBytes = readFileSync(finalizedEvidencePath);
  const evidenceDescriptor = {
    artifact_kind: "d2b-panel/validation-evidence",
    path: "evidence.md",
    sha256: createHash("sha256").update(finalizedEvidenceBytes).digest("hex"),
    size_bytes: finalizedEvidenceBytes.length,
  };

  const candidatePath = join(repo, "candidate.json");
  writeFileSync(
    candidatePath,
    `${JSON.stringify({
      program: "SPEC001",
      wave: "spec001w1",
      candidate_id: "candidate-1",
      content_id: "content-1",
      snapshot_sha256: "a".repeat(64),
      changed_paths: ["first.txt", literalBackslashPath],
    }, null, 2)}\n`,
  );
  const selectionPath = execFileSync(
    "node",
    [
      lifecycleScript,
      "select",
      candidatePath,
      "spec001w1",
      "--git-range",
      `${base}..${firstTip}`,
    ],
    { cwd: repo, encoding: "utf8" },
  ).trim();
  const discoveryRequestPath = join(repo, "discovery-request.json");
  execFileSync(
    "node",
    [lifecycleScript, "discovery-request", selectionPath, candidatePath, discoveryRequestPath],
    { cwd: repo, encoding: "utf8" },
  );

  console.log("stage-diffs: first review");
  const missingFinalizedEvidence = run(repo, [
    base,
    base,
    "spec001w1-r1",
    "--selection",
    selectionPath,
    "--candidate",
    candidatePath,
    "--discovery-request",
    discoveryRequestPath,
  ], { includeEvidence: false });
  check(
    "staging requires finalized validation evidence before completion",
    missingFinalizedEvidence.status === 2 &&
      /--evidence is required/.test(missingFinalizedEvidence.text) &&
      !existsSync(join(repo, ".scratch", "panel", "spec001w1-r1", ".complete")),
    missingFinalizedEvidence.text,
  );
  const missingDiscoveryRequest = run(repo, [
    base,
    base,
    "spec001w1-r1",
    "--selection",
    selectionPath,
    "--candidate",
    candidatePath,
  ]);
  check(
    "discovery staging refuses to complete without its supplied request",
    missingDiscoveryRequest.status === 2 &&
      /--discovery-request is required/.test(missingDiscoveryRequest.text) &&
      !existsSync(join(repo, ".scratch", "panel", "spec001w1-r1", ".complete")),
    missingDiscoveryRequest.text,
  );
  const originalDiscoveryRequest = readFileSync(discoveryRequestPath, "utf8");
  const crossCandidateDiscovery = JSON.parse(originalDiscoveryRequest);
  crossCandidateDiscovery.candidate.content_id = "stale-content";
  writeFileSync(
    discoveryRequestPath,
    `${JSON.stringify(crossCandidateDiscovery, null, 2)}\n`,
  );
  const staleDiscoveryRequest = run(repo, [
    base,
    base,
    "spec001w1-r1",
    "--selection",
    selectionPath,
    "--candidate",
    candidatePath,
    "--discovery-request",
    discoveryRequestPath,
  ]);
  check(
    "discovery staging rejects a cross-candidate request and removes only its owned packet",
    staleDiscoveryRequest.status === 2 &&
      /strict lifecycle validation/.test(staleDiscoveryRequest.text) &&
      !existsSync(join(repo, ".scratch", "panel", "spec001w1-r1")),
    staleDiscoveryRequest.text,
  );
  rmSync(join(repo, ".scratch", "panel", "spec001w1-r1"), {
    recursive: true,
    force: true,
  });
  writeFileSync(discoveryRequestPath, originalDiscoveryRequest);
  const selectionSymlink = join(repo, "selection-symlink.json");
  symlinkSync(selectionPath, selectionSymlink);
  const symlinkSelection = run(repo, [
    base,
    base,
    "spec001w1-r1",
    "--selection",
    selectionSymlink,
    "--candidate",
    candidatePath,
    "--discovery-request",
    discoveryRequestPath,
  ]);
  check(
    "staging rejects a symlinked lifecycle selection",
    symlinkSelection.status === 2 &&
      /symbolic link|ELOOP/.test(symlinkSelection.text) &&
      !existsSync(join(repo, ".scratch", "panel", "spec001w1-r1", ".complete")),
    symlinkSelection.text,
  );
  const selectionHardlink = join(repo, "selection-hardlink.json");
  linkSync(selectionPath, selectionHardlink);
  const hardlinkedSelection = run(repo, [
    base,
    base,
    "spec001w1-r1",
    "--selection",
    selectionHardlink,
    "--candidate",
    candidatePath,
    "--discovery-request",
    discoveryRequestPath,
  ]);
  check(
    "staging rejects a hardlinked lifecycle selection",
    hardlinkedSelection.status === 2 &&
      /link count|hardlink/.test(hardlinkedSelection.text) &&
      !existsSync(join(repo, ".scratch", "panel", "spec001w1-r1", ".complete")),
    hardlinkedSelection.text,
  );
  rmSync(selectionHardlink);
  const boundedPacket = run(repo, [
    base,
    base,
    "spec001w1-r1",
    "--selection",
    selectionPath,
    "--candidate",
    candidatePath,
    "--discovery-request",
    discoveryRequestPath,
  ], {
    env: { D2B_PANEL_LIFECYCLE_MAX_BYTES: "1" },
  });
  check(
    "staging preserves exact packets and removes an over-quota owned packet",
    boundedPacket.status === 2 &&
      /exact-packet quota/.test(boundedPacket.text) &&
      !existsSync(join(repo, ".scratch", "panel", "spec001w1-r1")),
    boundedPacket.text,
  );
  rmSync(join(repo, ".scratch", "panel", "spec001w1-r1"), {
    recursive: true,
    force: true,
  });
  const panelRoot = join(repo, ".scratch", "panel");
  const foreignLifecycle = join(panelRoot, "spec999w1-r1");
  mkdirSync(foreignLifecycle);
  writeFileSync(join(foreignLifecycle, ".complete"), "complete\n");
  const crossLifecycleQuota = run(repo, [
    base,
    base,
    "spec001w1-r1",
    "--selection",
    selectionPath,
    "--candidate",
    candidatePath,
    "--discovery-request",
    discoveryRequestPath,
  ], {
    env: { D2B_PANEL_LIFECYCLE_MAX_BYTES: "1" },
  });
  check(
    "root quota rejects another lifecycle before materializing the current round",
    crossLifecycleQuota.status === 2 &&
      /root-wide exact-packet quota/.test(crossLifecycleQuota.text) &&
      /before round materialization/.test(crossLifecycleQuota.text) &&
      !existsSync(join(panelRoot, "spec001w1-r1")) &&
      existsSync(foreignLifecycle),
    crossLifecycleQuota.text,
  );
  rmSync(foreignLifecycle, { recursive: true });

  const incompletePacket = join(panelRoot, "abandoned-r1");
  mkdirSync(incompletePacket);
  writeFileSync(join(incompletePacket, "partial.diff"), "partial\n");
  const incompleteQuota = run(repo, [
    base,
    base,
    "spec001w1-r1",
    "--selection",
    selectionPath,
    "--candidate",
    candidatePath,
    "--discovery-request",
    discoveryRequestPath,
  ], {
    env: { D2B_PANEL_LIFECYCLE_MAX_BYTES: "1" },
  });
  check(
    "root quota includes incomplete packets without deleting foreign state",
    incompleteQuota.status === 2 &&
      /root-wide exact-packet quota/.test(incompleteQuota.text) &&
      !existsSync(join(panelRoot, "spec001w1-r1")) &&
      existsSync(incompletePacket),
    incompleteQuota.text,
  );
  rmSync(incompletePacket, { recursive: true });

  const first = run(repo, [
    base,
    base,
    "spec001w1-r1",
    "--selection",
    selectionPath,
    "--candidate",
    candidatePath,
    "--discovery-request",
    discoveryRequestPath,
  ]);
  check("first review stages successfully", first.status === 0, first.text);

  const firstDir = join(repo, ".scratch", "panel", "spec001w1-r1");
  const firstAddress = JSON.parse(readFileSync(join(firstDir, "address.json"), "utf8"));
  const firstCompletion = JSON.parse(
    readFileSync(join(firstDir, ".complete"), "utf8"),
  );
  check(
    "first review records its lifecycle id",
    firstAddress.lifecycle_id === "spec001w1",
  );
  check("first review records its selection digest", typeof firstAddress.selection_sha256 === "string");
  check(
    "completion binds every reviewer-visible evidence packet",
    firstCompletion.schema_version === 2 &&
      firstCompletion.artifact_sha256["evidence.md"] ===
        evidenceDescriptor.sha256 &&
      firstCompletion.artifact_sha256["discovery-request.json"] ===
        createHash("sha256")
          .update(readFileSync(join(firstDir, "discovery-request.json")))
          .digest("hex") &&
      ["software", "test"].every((seat) =>
        typeof firstCompletion.artifact_sha256[
          `reviewer-notes/${seat}.md`
        ] === "string") &&
      typeof firstCompletion.artifact_sha256["review-request.md"] === "string" &&
      typeof firstCompletion.artifact_sha256["dispatch-prompt.txt"] === "string",
  );
  check(
    "generated discovery request canonically binds finalized evidence",
    JSON.parse(readFileSync(join(firstDir, "discovery-request.json"), "utf8"))
      .validation_evidence.some((entry) =>
        JSON.stringify(entry) === JSON.stringify(evidenceDescriptor)),
  );
  check("first review stages selection.json exactly", readFileSync(join(firstDir, "selection.json"), "utf8") === readFileSync(selectionPath, "utf8"));
  check("first review stages current-candidate.json exactly", readFileSync(join(firstDir, "current-candidate.json"), "utf8") === readFileSync(candidatePath, "utf8"));
  check(
    "git-range selection and staging preserve a literal backslash path",
    JSON.parse(readFileSync(join(firstDir, "selection.json"), "utf8"))
      .classification_inputs.changed_paths.includes(literalBackslashPath) &&
      JSON.parse(readFileSync(join(firstDir, "current-candidate.json"), "utf8"))
        .changed_paths.includes(literalBackslashPath),
  );
  const sourceDiscoveryRequest = JSON.parse(
    readFileSync(discoveryRequestPath, "utf8"),
  );
  const stagedDiscoveryRequest = JSON.parse(
    readFileSync(join(firstDir, "discovery-request.json"), "utf8"),
  );
  check(
    "first review preserves the generated request and adds only bound evidence",
    JSON.stringify({
      ...stagedDiscoveryRequest,
      validation_evidence: sourceDiscoveryRequest.validation_evidence,
    }) === JSON.stringify(sourceDiscoveryRequest) &&
      stagedDiscoveryRequest.validation_evidence.length ===
        sourceDiscoveryRequest.validation_evidence.length + 1,
  );
  check("first review writes its completion marker last", existsSync(join(firstDir, ".complete")));
  const firstRequest = readFileSync(join(firstDir, "review-request.md"), "utf8");
  const firstDispatch = readFileSync(join(firstDir, "dispatch-prompt.txt"), "utf8");
  check(
    "request names the exact delta range",
    firstRequest.includes(`Delta range: \`${base}..${firstTip}\``),
  );
  check(
    "request names the full context range",
    firstRequest.includes(`Full range: \`${base}..${firstTip}\``),
  );
  check(
    "request binds evidence and no-rerun instructions",
    firstRequest.includes("Validation evidence and phase deliverable") &&
      firstRequest.includes(
        "Do not rerun validation unless your seat-specific",
      ),
  );
  check(
    "first request asks for full-candidate comprehensive discovery",
    firstRequest.includes("full candidate") &&
      firstRequest.includes("every reasonably discoverable actionable finding"),
  );
  const malformedSelection = JSON.parse(readFileSync(selectionPath, "utf8"));
  malformedSelection.classification_inputs.unexpected = true;
  const validSelectionBytes = readFileSync(selectionPath, "utf8");
  writeFileSync(join(repo, "malformed-selection.json"), `${JSON.stringify(malformedSelection, null, 2)}\n`);
  const malformedStage = run(repo, [
    base,
    base,
    "spec001w1-r1",
    "--selection",
    join(repo, "malformed-selection.json"),
    "--candidate",
    candidatePath,
    "--discovery-request",
    discoveryRequestPath,
  ]);
  check(
    "staging rejects a malformed nested classification",
    malformedStage.status === 2 &&
      /unknown field/.test(malformedStage.text),
    malformedStage.text,
  );
  writeFileSync(selectionPath, validSelectionBytes);
  check(
    "first request is phase-aware and names discovery artifacts",
    firstRequest.includes("Phase: `discovery`") &&
      firstRequest.includes("Discovery request:") &&
      !firstRequest.includes("Immutable discovery ledger:"),
  );
  check(
    "dispatch prompt points at the complete request",
    firstDispatch.includes(join(firstDir, "review-request.md")),
  );
  check(
    "seat-specific note files are staged",
    ["software", "test"].every((seat) =>
      readFileSync(join(firstDir, "reviewer-notes", `${seat}.md`), "utf8")
        .includes(`Reviewer notes for ${seat}`),
    ),
  );
  const savedDiscoveryRequest = readFileSync(join(firstDir, "discovery-request.json"));
  rmSync(join(firstDir, "discovery-request.json"));
  const completeMissingCanonical = run(repo, [
    base,
    base,
    "spec001w1-r1",
    "--selection",
    selectionPath,
    "--candidate",
    candidatePath,
    "--discovery-request",
    discoveryRequestPath,
  ]);
  check(
    "a complete round never adds a missing canonical artifact",
    completeMissingCanonical.status === 2 &&
      /bound artifact discovery-request\.json is unavailable/.test(
        completeMissingCanonical.text,
      ) &&
      !existsSync(join(firstDir, "discovery-request.json")),
    completeMissingCanonical.text,
  );
  writeFileSync(join(firstDir, "discovery-request.json"), savedDiscoveryRequest);
  chmodSync(join(firstDir, "discovery-request.json"), 0o444);

  const stagedEvidencePath = join(firstDir, "evidence.md");
  const originalEvidenceBytes = readFileSync(stagedEvidencePath);
  chmodSync(stagedEvidencePath, 0o644);
  writeFileSync(stagedEvidencePath, "mutated evidence\n");
  const mutatedEvidence = run(repo, [
    base,
    base,
    "spec001w1-r1",
    "--selection",
    selectionPath,
    "--candidate",
    candidatePath,
    "--discovery-request",
    discoveryRequestPath,
  ]);
  check(
    "post-completion evidence mutation is refused",
    mutatedEvidence.status === 2 &&
      /post-completion mutation of evidence\.md is refused/.test(
        mutatedEvidence.text,
      ),
    mutatedEvidence.text,
  );
  writeFileSync(stagedEvidencePath, originalEvidenceBytes);
  chmodSync(stagedEvidencePath, 0o444);

  const softwareNote = join(firstDir, "reviewer-notes", "software.md");
  const originalSoftwareNote = readFileSync(softwareNote);
  chmodSync(softwareNote, 0o644);
  writeFileSync(softwareNote, "mutated reviewer note\n");
  const mutatedNote = run(repo, [
    base,
    base,
    "spec001w1-r1",
    "--selection",
    selectionPath,
    "--candidate",
    candidatePath,
    "--discovery-request",
    discoveryRequestPath,
  ]);
  check(
    "post-completion reviewer-note mutation is refused",
    mutatedNote.status === 2 &&
      /post-completion mutation of reviewer-notes\/software\.md is refused/.test(
        mutatedNote.text,
      ),
    mutatedNote.text,
  );
  writeFileSync(softwareNote, originalSoftwareNote);
  chmodSync(softwareNote, 0o444);

  const originalDeltaBytes = readFileSync(join(firstDir, "delta.diff"), "utf8");
  chmodSync(join(firstDir, "delta.diff"), 0o644);
  writeFileSync(join(firstDir, "delta.diff"), "conflicting scratch bytes\n");
  const firstConflict = run(repo, [
    base,
    base,
    "spec001w1-r1",
    "--selection",
    selectionPath,
    "--candidate",
    candidatePath,
    "--discovery-request",
    discoveryRequestPath,
  ]);
  check(
    "completion validation refuses mutation before generated-byte comparison",
    firstConflict.status === 2 &&
      /post-completion mutation of delta\.diff is refused/.test(
        firstConflict.text,
      ),
    firstConflict.text,
  );
  writeFileSync(join(firstDir, "delta.diff"), originalDeltaBytes);
  chmodSync(join(firstDir, "delta.diff"), 0o444);

  const dispatchPath = join(firstDir, "dispatch-prompt.txt");
  const originalDispatchBytes = readFileSync(dispatchPath);
  const dispatchAlias = join(repo, "dispatch-alias.txt");
  writeFileSync(dispatchAlias, originalDispatchBytes);
  rmSync(dispatchPath);
  linkSync(dispatchAlias, dispatchPath);
  const hardlinkedDispatch = run(repo, [
    base,
    base,
    "spec001w1-r1",
    "--selection",
    selectionPath,
    "--candidate",
    candidatePath,
    "--discovery-request",
    discoveryRequestPath,
  ]);
  check(
    "dispatch consumption rejects an identical-byte hardlink substitution",
    hardlinkedDispatch.status === 2 &&
      /link count|hardlink/.test(hardlinkedDispatch.text),
    hardlinkedDispatch.text,
  );
  rmSync(dispatchPath);
  writeFileSync(dispatchPath, originalDispatchBytes);
  chmodSync(dispatchPath, 0o444);

  const firstRoster = JSON.parse(readFileSync(selectionPath, "utf8")).roster;
  for (const seat of firstRoster) {
    writeFileSync(
      join(firstDir, "verdicts", `${seat}.json`),
      `${JSON.stringify({
        engineer: seat,
        signoff: true,
        summary: "Reviewed.",
        recommendations: [],
      })}\n`,
    );
  }
  const addressPath = join(firstDir, "address.json");
  const originalAddressBytes = readFileSync(addressPath);
  const mutatedAddress = JSON.parse(originalAddressBytes);
  delete mutatedAddress.phase;
  delete mutatedAddress.selection_sha256;
  chmodSync(addressPath, 0o644);
  writeFileSync(
    addressPath,
    `${JSON.stringify(mutatedAddress, null, 2)}\n`,
  );
  const postCompletionAddressMutation = run(repo, [
    base,
    base,
    "spec001w1-r1",
    "--selection",
    selectionPath,
    "--candidate",
    candidatePath,
    "--discovery-request",
    discoveryRequestPath,
  ]);
  check(
    "post-completion address compatibility edits are refused",
    postCompletionAddressMutation.status === 2 &&
      /post-completion mutation of address\.json is refused/.test(
        postCompletionAddressMutation.text,
      ),
    postCompletionAddressMutation.text,
  );
  writeFileSync(addressPath, originalAddressBytes);
  chmodSync(addressPath, 0o444);

  writeFileSync(join(repo, "Makefile"), "second build change\n");
  git(repo, "add", "Makefile");
  git(repo, "commit", "--quiet", "-m", "second");
  const secondTip = git(repo, "rev-parse", "HEAD");

  const currentCandidatePath = join(repo, "current-candidate.json");
  writeFileSync(
    currentCandidatePath,
    `${JSON.stringify({
      program: "SPEC001",
      wave: "spec001w1",
      candidate_id: "candidate-2",
      content_id: "content-2",
      snapshot_sha256: "b".repeat(64),
      changed_paths: ["Makefile", "first.txt", literalBackslashPath],
    }, null, 2)}\n`,
  );
  const deltaPath = join(repo, "fix-delta.json");
  writeFileSync(deltaPath, `${JSON.stringify({ changed_paths: ["Makefile"] }, null, 2)}\n`);
  const currentSelectionPath = execFileSync(
    "node",
    [
      lifecycleScript,
      "select",
      currentCandidatePath,
      "spec001w1",
      "--phase",
      "verification",
      "--previous-selection",
      selectionPath,
      "--fix-delta",
      deltaPath,
      "--git-range",
      `${base}..${secondTip}`,
    ],
    { cwd: repo, encoding: "utf8" },
  ).trim();
  const stagedLedger = join(repo, "source-ledger.json");
  const stagedResponses = join(repo, "source-responses.json");
  const stagedSelfVerification = join(repo, "source-self-verification.json");
  const discoveryResultsPath = join(repo, "discovery-results.json");
  const groupsPath = join(repo, "discovery-groups.json");
  writeFileSync(
    join(firstDir, "verdicts", "software.json"),
    `${JSON.stringify({
      engineer: "software",
      signoff: false,
      summary: "The fix needs verification.",
      recommendations: [{
        severity: "high",
        where: "Makefile",
        what: "The fix needs verification.",
        why: "The change could regress the handoff.",
        fix: "Verify the fix.",
      }],
    }, null, 2)}\n`,
  );
  execFileSync(
    "node",
    [
      lifecycleScript,
      "adapt-discovery",
      join(firstDir, "verdicts"),
      discoveryResultsPath,
      "--selection",
      selectionPath,
      "--candidate",
      candidatePath,
    ],
    { cwd: repo, encoding: "utf8" },
  );
  writeFileSync(
    groupsPath,
    `${JSON.stringify([{
      source_finding_ids: ["software:1"],
      description: "The fix needs verification.",
      severity: "MAJOR",
      impact: "The change could regress the handoff.",
      recommendation: "Verify the fix.",
    }], null, 2)}\n`,
  );
  execFileSync(
    "node",
    [
      lifecycleScript,
      "merge-ledger",
      selectionPath,
      discoveryResultsPath,
      groupsPath,
      stagedLedger,
      "--candidate",
      candidatePath,
    ],
    { cwd: repo, encoding: "utf8" },
  );
  execFileSync(
    "node",
    [lifecycleScript, "response-template", stagedLedger, stagedResponses],
    { cwd: repo, encoding: "utf8" },
  );
  const responseEnvelope = JSON.parse(readFileSync(stagedResponses, "utf8"));
  responseEnvelope.responses[0] = {
    issue_id: "R1",
    disposition: "Fixed",
    changed_surface: ["Makefile"],
    justification: "The fix is staged for verification.",
    evidence: "focused test",
  };
  writeFileSync(stagedResponses, `${JSON.stringify(responseEnvelope, null, 2)}\n`);
  writeFileSync(
    stagedSelfVerification,
    `${JSON.stringify({
      tests: ["focused stage test"],
      lint: "passed",
      formatting: "passed",
      static_analysis: "passed",
      build: "not applicable",
      uncovered_areas: ["none"],
      self_review: "passed",
    }, null, 2)}\n`,
  );
  const verificationSourceDir = join(repo, "verification-requests");
  execFileSync(
    "node",
    [
      lifecycleScript,
      "verification",
      currentSelectionPath,
      stagedLedger,
      stagedResponses,
      stagedSelfVerification,
      verificationSourceDir,
      "--candidate",
      currentCandidatePath,
      "--prior-selection",
      selectionPath,
      "--prior-verdicts",
      join(firstDir, "verdicts"),
      "--delta",
      deltaPath,
    ],
    { cwd: repo, encoding: "utf8" },
  );
  const verificationRoster = JSON.parse(
    readFileSync(currentSelectionPath, "utf8"),
  ).roster;

  console.log("stage-diffs: fail-closed continuity");
  const wrongPreviousTip = run(repo, [
    base,
    base,
    "spec001w1-r2",
    "--selection",
    currentSelectionPath,
    "--candidate",
    currentCandidatePath,
    "--ledger",
    stagedLedger,
    "--responses",
    stagedResponses,
    "--self-verification",
    stagedSelfVerification,
    "--verification-dir",
    verificationSourceDir,
  ]);
  check(
    "later review rejects a non-incremental previous tip",
    wrongPreviousTip.status === 2 &&
      wrongPreviousTip.text.includes(
        "incremental range does not start at the previous recorded tip",
      ),
    wrongPreviousTip.text,
  );

  const missingVerdict = join(firstDir, "verdicts", "test.json");
  const savedVerdict = readFileSync(missingVerdict);
  rmSync(missingVerdict);
  const incompletePreviousReview = run(repo, [
    base,
    firstTip,
    "spec001w1-r2",
    "--selection",
    currentSelectionPath,
    "--candidate",
    currentCandidatePath,
    "--ledger",
    stagedLedger,
    "--responses",
    stagedResponses,
    "--self-verification",
    stagedSelfVerification,
    "--verification-dir",
    verificationSourceDir,
  ]);
  check(
    "later review rejects a missing prior seat verdict",
    incompletePreviousReview.status === 2 &&
      incompletePreviousReview.text.includes(
      "missing previous verdict for incumbent seat test",
      ),
    incompletePreviousReview.text,
  );
  writeFileSync(missingVerdict, savedVerdict);

  console.log("stage-diffs: incremental review");
  const missingVerificationDirectory = run(repo, [
    base,
    firstTip,
    "spec001w1-r2",
    "--selection",
    currentSelectionPath,
    "--candidate",
    currentCandidatePath,
    "--ledger",
    stagedLedger,
    "--responses",
    stagedResponses,
    "--self-verification",
    stagedSelfVerification,
  ]);
  check(
    "verification staging refuses to complete without every seat request",
    missingVerificationDirectory.status === 2 &&
      /--verification-dir is required/.test(missingVerificationDirectory.text) &&
      !existsSync(join(repo, ".scratch", "panel", "spec001w1-r2", ".complete")),
    missingVerificationDirectory.text,
  );
  const incompleteVerificationDir = join(repo, "incomplete-verification-requests");
  mkdirSync(incompleteVerificationDir);
  for (const seat of verificationRoster.slice(0, -1)) {
    writeFileSync(
      join(incompleteVerificationDir, `${seat}.json`),
      `${JSON.stringify({ seat, request: "incomplete" })}\n`,
    );
  }
  const incompleteVerification = run(repo, [
    base,
    firstTip,
    "spec001w1-r2",
    "--selection",
    currentSelectionPath,
    "--candidate",
    currentCandidatePath,
    "--ledger",
    stagedLedger,
    "--responses",
    stagedResponses,
    "--self-verification",
    stagedSelfVerification,
    "--verification-dir",
    incompleteVerificationDir,
  ]);
  check(
    "verification staging refuses an incomplete per-seat request directory",
    incompleteVerification.status === 2 &&
      /exactly one readable JSON request per selected seat/.test(incompleteVerification.text) &&
      !existsSync(join(repo, ".scratch", "panel", "spec001w1-r2", ".complete")),
    incompleteVerification.text,
  );
  const currentSelection = JSON.parse(
    readFileSync(currentSelectionPath, "utf8"),
  );
  const badFullSelection = JSON.parse(JSON.stringify(currentSelection));
  badFullSelection.classification_inputs.changed_paths = ["Makefile"];
  badFullSelection.classification_inputs.full_candidate.changed_paths = ["Makefile"];
  badFullSelection.classification_inputs.fix_delta.changed_paths = ["Makefile"];
  const badFullSelectionPath = join(repo, "bad-full-selection.json");
  writeFileSync(badFullSelectionPath, `${JSON.stringify(badFullSelection, null, 2)}\n`);
  const fullRangeMismatch = run(repo, [
    base,
    firstTip,
    "spec001w1-r2",
    "--selection",
    badFullSelectionPath,
    "--candidate",
    currentCandidatePath,
    "--ledger",
    stagedLedger,
    "--responses",
    stagedResponses,
    "--self-verification",
    stagedSelfVerification,
    "--verification-dir",
    verificationSourceDir,
  ]);
  check(
    "verification staging compares base-to-tip full paths",
    fullRangeMismatch.status === 2 &&
      /full-candidate paths do not match git range/.test(fullRangeMismatch.text) &&
      !existsSync(join(repo, ".scratch", "panel", "spec001w1-r2", ".complete")),
    fullRangeMismatch.text,
  );
  const badDeltaSelection = JSON.parse(JSON.stringify(currentSelection));
  badDeltaSelection.classification_inputs.fix_delta.changed_paths = ["first.txt"];
  const badDeltaSelectionPath = join(repo, "bad-delta-selection.json");
  writeFileSync(badDeltaSelectionPath, `${JSON.stringify(badDeltaSelection, null, 2)}\n`);
  const deltaRangeMismatch = run(repo, [
    base,
    firstTip,
    "spec001w1-r2",
    "--selection",
    badDeltaSelectionPath,
    "--candidate",
    currentCandidatePath,
    "--ledger",
    stagedLedger,
    "--responses",
    stagedResponses,
    "--self-verification",
    stagedSelfVerification,
    "--verification-dir",
    verificationSourceDir,
  ]);
  check(
    "verification staging compares previous-tip-to-tip delta paths",
    deltaRangeMismatch.status === 2 &&
      /fix-delta paths do not match git range/.test(deltaRangeMismatch.text) &&
      !existsSync(join(repo, ".scratch", "panel", "spec001w1-r2", ".complete")),
    deltaRangeMismatch.text,
  );
  const invalidVerificationDirectory = (name, mutate) => {
    const directory = join(repo, name);
    mkdirSync(directory);
    for (const seat of verificationRoster) {
      const request = JSON.parse(
        readFileSync(join(verificationSourceDir, `${seat}.json`), "utf8"),
      );
      if (seat === "software") mutate(request);
      writeFileSync(directory + `/${seat}.json`, `${JSON.stringify(request, null, 2)}\n`);
    }
    return directory;
  };
  const rejectsStagedVerification = (name, mutate) => {
    const directory = invalidVerificationDirectory(name, mutate);
    const result = run(repo, [
      base,
      firstTip,
      "spec001w1-r2",
      "--selection",
      currentSelectionPath,
      "--candidate",
      currentCandidatePath,
      "--ledger",
      stagedLedger,
      "--responses",
      stagedResponses,
      "--self-verification",
      stagedSelfVerification,
      "--verification-dir",
      directory,
    ]);
    check(
      name,
      result.status === 2 &&
        /strict lifecycle validation/.test(result.text) &&
        !existsSync(join(repo, ".scratch", "panel", "spec001w1-r2", ".complete")),
      result.text,
    );
    rmSync(join(repo, ".scratch", "panel", "spec001w1-r2"), {
      recursive: true,
      force: true,
    });
  };
  rejectsStagedVerification(
    "verification staging rejects a well-formed cross-candidate request",
    (request) => {
      request.current_candidate.candidate_id = "stale-candidate";
    },
  );
  rejectsStagedVerification(
    "verification staging rejects a well-formed cross-ledger request",
    (request) => {
      request.ledger.candidate_id = "foreign-candidate";
    },
  );
  rejectsStagedVerification(
    "verification staging rejects a well-formed cross-selection profile",
    (request) => {
      request.selection.profiles.software = ["javascript"];
    },
  );
  rejectsStagedVerification(
    "verification staging rejects a well-formed stale response",
    (request) => {
      request.responses[0].justification = "stale response from an earlier ledger";
    },
  );
  rejectsStagedVerification(
    "verification staging rejects removed full context",
    (request) => {
      request.full_context = {
        candidate: "context from an earlier candidate",
        validation: "stale evidence",
      };
    },
  );
  rejectsStagedVerification(
    "verification staging rejects removed actual delta context",
    (request) => {
      request.actual_delta.context = {
        changed_paths: ["Makefile"],
      };
    },
  );
  rejectsStagedVerification(
    "verification staging rejects a swapped request seat",
    (request) => {
      request.seat = "test";
    },
  );
  rejectsStagedVerification(
    "verification staging rejects stale prior selection",
    (request) => {
      request.prior_selection.content_id = "stale-prior-content";
    },
  );
  rejectsStagedVerification(
    "verification staging rejects a null incumbent prior status",
    (request) => {
      request.previous_status = null;
    },
  );
  rejectsStagedVerification(
    "verification staging rejects stale incumbent prior status",
    (request) => {
      request.previous_status = {
        engineer: "software",
        signoff: false,
        summary: "status from a different prior review",
        recommendations: ["stale"],
      };
    },
  );
  const second = run(repo, [
    base,
    firstTip,
    "spec001w1-r2",
    "--selection",
    currentSelectionPath,
    "--candidate",
    currentCandidatePath,
    "--ledger",
    stagedLedger,
    "--responses",
    stagedResponses,
    "--self-verification",
    stagedSelfVerification,
    "--verification-dir",
    verificationSourceDir,
  ]);
  check("later review stages successfully", second.status === 0, second.text);

  const secondDir = join(repo, ".scratch", "panel", "spec001w1-r2");
  check("later review stages selection.json exactly", readFileSync(join(secondDir, "selection.json"), "utf8") === readFileSync(currentSelectionPath, "utf8"));
  check("later review stages current-candidate.json exactly", readFileSync(join(secondDir, "current-candidate.json"), "utf8") === readFileSync(currentCandidatePath, "utf8"));
  check("later review stages the immutable ledger exactly", readFileSync(join(secondDir, "discovery-ledger.json"), "utf8") === readFileSync(stagedLedger, "utf8"));
  check("later review stages responses exactly", readFileSync(join(secondDir, "responses.json"), "utf8") === readFileSync(stagedResponses, "utf8"));
  check("later review stages self-verification exactly", readFileSync(join(secondDir, "self-verification.json"), "utf8") === readFileSync(stagedSelfVerification, "utf8"));
  check(
    "later review stages every verification request exactly",
    verificationRoster.every((seat) =>
      readFileSync(join(secondDir, "verification", `${seat}.json`), "utf8") ===
        readFileSync(join(verificationSourceDir, `${seat}.json`), "utf8")),
  );
  check(
    "newly selected seats carry a null prior status",
    JSON.parse(readFileSync(join(secondDir, "verification", "build.json"), "utf8"))
      .previous_status === null,
  );
  check("later review writes a completion marker", existsSync(join(secondDir, ".complete")));
  const delta = readFileSync(join(secondDir, "delta.diff"), "utf8");
  const full = readFileSync(join(secondDir, "full.diff"), "utf8");
  const secondRequest = readFileSync(join(secondDir, "review-request.md"), "utf8");
  check(
    "incremental diff excludes earlier changed paths",
    delta.includes("Makefile") && !delta.includes("first.txt"),
  );
  check(
    "full diff retains all branch changes",
    full.includes("first.txt") && full.includes("Makefile"),
  );
  check(
    "later request names the prior verdict and invalidated sign-off",
    secondRequest.includes(
      join(firstDir, "verdicts", "<your-seat>.json"),
    ) &&
      /Any content change invalidated every prior\s+sign-off/.test(
        secondRequest,
      ),
  );
  check(
    "later request names the exact incremental range",
    secondRequest.includes(`Delta range: \`${firstTip}..${secondTip}\``),
  );
  check(
    "later request names verification artifacts and allows the new seat",
    secondRequest.includes("Phase: `verification`") &&
      secondRequest.includes("Immutable discovery ledger:") &&
      secondRequest.includes("Approval output after verdict collection:") &&
      !secondRequest.includes("missing previous verdict for seat build"),
  );
  check(
    "new-seat request makes the absent prior verdict explicit",
    secondRequest.includes("newly selected seat") &&
      secondRequest.includes("no prior verdict exists"),
  );
  check(
    "new-seat note carries its first-verification obligation",
    readFileSync(join(secondDir, "reviewer-notes", "build.md"), "utf8").includes(
      "No prior verdict exists for this seat",
    ) &&
      readFileSync(join(secondDir, "reviewer-notes", "build.md"), "utf8").includes(
        "complete ledger",
      ),
  );

  for (const seat of verificationRoster) {
    writeFileSync(
      join(secondDir, "verdicts", `${seat}.json`),
      `${JSON.stringify({
        engineer: seat,
        signoff: true,
        summary: "Verified.",
        recommendations: [],
      })}\n`,
    );
  }
  const noOpDeltaPath = join(repo, "no-op-delta.json");
  writeFileSync(
    noOpDeltaPath,
    `${JSON.stringify({ changed_paths: [] }, null, 2)}\n`,
  );
  const noOpSelectionPath = join(repo, "no-op-selection.json");
  execFileSync(
    "node",
    [
      "--input-type=module",
      "-e",
      `
import fs from "node:fs";
import { pathToFileURL } from "node:url";
const [helperPath, candidatePath, priorPath, outputPath] = process.argv.slice(2);
const { createSelection, readSelection } =
  await import(pathToFileURL(helperPath).href);
const candidate = JSON.parse(fs.readFileSync(candidatePath, "utf8"));
createSelection({
  ...candidate,
  lifecycle_id: "spec001w1",
  phase: "verification",
  previous_selection: readSelection(priorPath),
}, { path: outputPath });
`,
      "d2b-test",
      lifecycleScript,
      currentCandidatePath,
      currentSelectionPath,
      noOpSelectionPath,
    ],
    { cwd: repo, encoding: "utf8" },
  );
  const noOpVerificationDir = join(repo, "no-op-verification-requests");
  execFileSync(
    "node",
    [
      lifecycleScript,
      "verification",
      noOpSelectionPath,
      stagedLedger,
      stagedResponses,
      stagedSelfVerification,
      noOpVerificationDir,
      "--candidate",
      currentCandidatePath,
      "--prior-selection",
      currentSelectionPath,
      "--prior-verdicts",
      join(secondDir, "verdicts"),
      "--delta",
      noOpDeltaPath,
    ],
    { cwd: repo, encoding: "utf8" },
  );
  const noOp = run(repo, [
    base,
    secondTip,
    "spec001w1-r3",
    "--selection",
    noOpSelectionPath,
    "--candidate",
    currentCandidatePath,
    "--ledger",
    stagedLedger,
    "--responses",
    stagedResponses,
    "--self-verification",
    stagedSelfVerification,
    "--verification-dir",
    noOpVerificationDir,
  ]);
  const noOpDir = join(repo, ".scratch", "panel", "spec001w1-r3");
  check(
    "verification permits an unchanged-tip no-op with an exact empty delta",
    noOp.status === 0 &&
      readFileSync(join(noOpDir, "delta.diff")).length === 0 &&
      verificationRoster.every((seat) =>
        readFileSync(join(noOpDir, "verification", `${seat}.json`), "utf8") ===
          readFileSync(join(noOpVerificationDir, `${seat}.json`), "utf8")),
    noOp.text,
  );

  const reused = run(repo, [
    base,
    firstTip,
    "spec001w1-r2",
    "--selection",
    currentSelectionPath,
    "--candidate",
    currentCandidatePath,
    "--ledger",
    stagedLedger,
    "--responses",
    stagedResponses,
    "--self-verification",
    stagedSelfVerification,
    "--verification-dir",
    verificationSourceDir,
  ]);
  check(
    "a review id with verdicts cannot be restaged",
    reused.status === 2 &&
      reused.text.includes("already has verdicts"),
    reused.text,
  );
  rmSync(join(firstDir, ".complete"));
  const incompleteRetry = run(repo, [
    base,
    base,
    "spec001w1-r1",
    "--selection",
    selectionPath,
    "--candidate",
    candidatePath,
    "--discovery-request",
    discoveryRequestPath,
  ]);
  check(
    "an unmarked scratch directory is non-authoritative and names cleanup",
    incompleteRetry.status === 2 &&
      /non-authoritative/.test(incompleteRetry.text) &&
      /rm -rf/.test(incompleteRetry.text),
    incompleteRetry.text,
  );

  const c1Path = `c1-${"\u0080"}.txt`;
  writeFileSync(join(repo, c1Path), "c1 control character\n");
  git(repo, "add", c1Path);
  git(repo, "commit", "--quiet", "-m", "c1 path");
  const c1Selection = spawnSync(
    "node",
    [
      lifecycleScript,
      "select",
      currentCandidatePath,
      "spec001w1",
      "--phase",
      "verification",
      "--previous-selection",
      selectionPath,
      "--fix-delta",
      deltaPath,
      "--git-range",
      `${secondTip}..HEAD`,
    ],
    { cwd: repo, encoding: "utf8" },
  );
  check(
    "git-range selection refuses a C1 path",
    c1Selection.status !== 0 &&
      /control character/.test(`${c1Selection.stdout}${c1Selection.stderr}`),
    `${c1Selection.stdout}${c1Selection.stderr}`,
  );
  const c1Staging = run(repo, [
    base,
    secondTip,
    "spec001w1-r4",
    "--selection",
    currentSelectionPath,
    "--candidate",
    currentCandidatePath,
    "--ledger",
    stagedLedger,
    "--responses",
    stagedResponses,
    "--self-verification",
    stagedSelfVerification,
    "--verification-dir",
    verificationSourceDir,
  ]);
  check(
    "staging refuses a git range containing a C1 path",
    c1Staging.status === 2 &&
      /control character/.test(c1Staging.text) &&
      !existsSync(join(repo, ".scratch", "panel", "spec001w1-r4", ".complete")),
    c1Staging.text,
  );
} finally {
  rmSync(repo, { recursive: true, force: true });
}

if (failures > 0) {
  console.error(`\ntest-stage-diffs: ${failures} failure(s)`);
  process.exit(1);
}
console.log("\ntest-stage-diffs: all cases passed");
