#!/usr/bin/env node
// Coverage for the staged panel review request. The test keeps the packet
// contract focused on ordinary create-or-compare files and the required
// predecessor completion marker.

import {
  chmodSync,
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { createHash } from "node:crypto";
import {
  createVerificationResultArtifact,
  writeFinalizeHandoff,
  stableStringify,
  writeAdvanceVerification,
} from "../../.github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs";
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
const dispatchPolicy = join(
  root,
  ".github",
  "skills",
  "d2b-panel-round",
  "dispatch-policy.json",
);
const allSeats = [
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
  "networking",
  "kernel",
  "build",
];

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

function writeJson(path, value) {
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
}

function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

function digest(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function stageArgs(
  base,
  previousTip,
  round,
  selection,
  candidate,
  request,
  lifecycle = "spec001w1",
) {
  return [
    base,
    previousTip,
    round,
    "--lifecycle",
    lifecycle,
    "--selection",
    selection,
    "--candidate",
    candidate,
    "--discovery-request",
    request,
  ];
}

const repo = mkdtempSync(join(tmpdir(), "d2b-stage-diffs-"));
try {
  const stageSource = readFileSync(script, "utf8");
  check(
    "staging uses ordinary filesystem publication",
    !/flock|proc\/self\/fd|O_TMPFILE|linkat|renameat2|fsync|fdatasync|retention|quota|NoFollow/.test(
      stageSource,
    ) &&
      stageSource.includes("renameSync") &&
      stageSource.includes("writeFileSync"),
  );
  check(
    "discovery incrementally scans bounded packet entries",
    stageSource.includes("opendirSync") &&
      stageSource.includes("readSync()") &&
      stageSource.includes("lstatSync(path.join(directory, entry.name))") &&
      stageSource.includes("entryKind") &&
      stageSource.includes("canonicalFileNames") &&
      stageSource.includes("canonicalContentDirectories") &&
      stageSource.includes('failScan("canonical-remnant"') &&
      !stageSource.includes("readdirSync(panelRoot)"),
  );

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
  cpSync(
    selectionTable,
    join(repo, ".github", "skills", "d2b-panel-round", "selection-table.json"),
  );
  cpSync(
    dispatchPolicy,
    join(repo, ".github", "skills", "d2b-panel-round", "dispatch-policy.json"),
  );
  for (const seat of allSeats) {
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
    sha256: digest(finalizedEvidenceBytes),
    size_bytes: finalizedEvidenceBytes.length,
  };

  const candidatePath = join(repo, "candidate.json");
  writeJson(candidatePath, {
    program: "SPEC001",
    wave: "spec001w1",
    candidate_id: "a".repeat(64),
    content_id: "b".repeat(64),
    snapshot_sha256: "c".repeat(64),
    changed_paths: ["first.txt", literalBackslashPath],
  });
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
    [
      lifecycleScript,
      "discovery-request",
      selectionPath,
      candidatePath,
      discoveryRequestPath,
    ],
    { cwd: repo, encoding: "utf8" },
  );

  console.log("stage-diffs: first review");
  const missingFinalizedEvidence = run(
    repo,
    stageArgs(base, base, "spec001w1-r1", selectionPath, candidatePath, discoveryRequestPath),
    { includeEvidence: false },
  );
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
  const staleRequest = readJson(discoveryRequestPath);
  staleRequest.candidate.content_id = "d".repeat(64);
  writeJson(discoveryRequestPath, staleRequest);
  const staleDiscoveryRequest = run(
    repo,
    stageArgs(base, base, "spec001w1-r1", selectionPath, candidatePath, discoveryRequestPath),
  );
  check(
    "a stale request is rejected without silently replacing its packet",
    staleDiscoveryRequest.status === 2 &&
      /strict lifecycle validation/.test(staleDiscoveryRequest.text) &&
      existsSync(join(repo, ".scratch", "panel", "spec001w1-r1")) &&
      !existsSync(join(repo, ".scratch", "panel", "spec001w1-r1", ".complete")),
    staleDiscoveryRequest.text,
  );
  rmSync(join(repo, ".scratch", "panel", "spec001w1-r1"), {
    recursive: true,
    force: true,
  });
  writeFileSync(discoveryRequestPath, originalDiscoveryRequest);

  const first = run(
    repo,
    stageArgs(base, base, "spec001w1-r1", selectionPath, candidatePath, discoveryRequestPath),
  );
  check("first review stages successfully", first.status === 0, first.text);
  const firstDir = join(repo, ".scratch", "panel", "spec001w1-r1");
  const firstAddress = readJson(join(firstDir, "address.json"));
  const firstCompletion = readJson(join(firstDir, ".complete"));
  const firstRoster = readJson(selectionPath).roster;
  const firstRequest = readFileSync(join(firstDir, "review-request.md"), "utf8");
  const firstDispatch = readFileSync(join(firstDir, "dispatch-prompt.txt"), "utf8");
  check(
    "first review records its lifecycle id and ordinary display paths",
    firstAddress.lifecycle_id === "spec001w1" &&
      firstAddress.selection_path === join(firstDir, "selection.json") &&
      !JSON.stringify(firstAddress).includes("/proc/self/fd/") &&
      !firstRequest.includes("/proc/self/fd/") &&
      !firstDispatch.includes("/proc/self/fd/"),
  );
  check(
    "completion binds canonical evidence with size and digest",
    firstCompletion.schema_version === 4 &&
      firstCompletion.artifact_sha256["evidence.md"] === evidenceDescriptor.sha256 &&
      firstCompletion.artifact_bytes["evidence.md"] === evidenceDescriptor.size_bytes &&
      firstCompletion.artifact_sha256["selection.json"] ===
        digest(readFileSync(join(firstDir, "selection.json"))) &&
      firstCompletion.artifact_sha256["dispatch-binding.json"] ===
        digest(readFileSync(join(firstDir, "dispatch-binding.json"))) &&
      firstRoster.every((seat) =>
        firstCompletion.artifact_sha256[
          `agent-definitions/panel-${seat}.agent.md`
        ] === digest(readFileSync(join(agents, `panel-${seat}.agent.md`))) &&
        firstCompletion.artifact_bytes[
          `agent-definitions/panel-${seat}.agent.md`
        ] === readFileSync(join(agents, `panel-${seat}.agent.md`)).length,
      ),
  );
  check(
    "dispatch binding is projected for exactly the selected roster",
    readJson(join(firstDir, "dispatch-binding.json")).roster.join(",") ===
      firstRoster.join(",") &&
      Object.keys(readJson(join(firstDir, "dispatch-binding.json")).bindings)
        .sort()
        .join(",") === firstRoster.slice().sort().join(","),
  );
  check(
    "generated discovery request binds finalized evidence",
    readJson(join(firstDir, "discovery-request.json")).validation_evidence.some(
      (entry) => JSON.stringify(entry) === JSON.stringify(evidenceDescriptor),
    ),
  );
  check(
    "first review stages the selection and candidate exactly",
    readFileSync(join(firstDir, "selection.json"), "utf8") ===
      readFileSync(selectionPath, "utf8") &&
      readFileSync(join(firstDir, "current-candidate.json"), "utf8") ===
        readFileSync(candidatePath, "utf8") &&
      readJson(join(firstDir, "selection.json")).classification_inputs.changed_paths.includes(
        literalBackslashPath,
      ),
  );
  check(
    "discovery request carries the distinct four-field verdict contract",
    firstRequest.includes("discovery verdict has exactly four top-level fields") &&
      firstRequest.includes("does not contain") &&
      firstRequest.includes("`verified_issue_statuses` or `late_findings`"),
  );
  check(
    "dispatch prompt points at the bound panel definition snapshot",
    firstDispatch.includes(
      join(
        firstDir,
        "agent-definitions",
        "panel-<your-seat>.agent.md",
      ),
    ) &&
      firstRequest.includes(
        join(
          firstDir,
          "agent-definitions",
          "panel-<your-seat>.agent.md",
        ),
      ) &&
      firstRoster.every((seat) =>
        readFileSync(
          join(firstDir, "agent-definitions", `panel-${seat}.agent.md`),
          "utf8",
        ) === readFileSync(join(agents, `panel-${seat}.agent.md`), "utf8"),
      ),
  );

  const savedFirstCompletionMarker = readFileSync(join(firstDir, ".complete"));
  rmSync(join(firstDir, ".complete"));
  const rewritePacketBinding = (packet, relativePath) => {
    const markerPath = join(packet, ".complete");
    const marker = readJson(markerPath);
    const bytes = readFileSync(join(packet, relativePath));
    marker.artifact_sha256[relativePath] = digest(bytes);
    marker.artifact_bytes[relativePath] = bytes.length;
    chmodSync(markerPath, 0o644);
    writeJson(markerPath, marker);
    chmodSync(markerPath, 0o444);
  };
  const copyCompletedDiscoveryPacket = (name, relocateSelection = true) => {
    const packet = join(repo, ".scratch", "panel", name);
    cpSync(firstPacketTemplateDir, packet, { recursive: true });
    if (existsSync(join(packet, ".complete"))) {
      chmodSync(join(packet, ".complete"), 0o644);
    }
    writeFileSync(join(packet, ".complete"), savedFirstCompletionMarker);
    chmodSync(join(packet, ".complete"), 0o444);
    if (relocateSelection) {
      const addressPath = join(packet, "address.json");
      const address = readJson(addressPath);
      chmodSync(addressPath, 0o644);
      address.selection_path = join(packet, "selection.json");
      writeJson(addressPath, address);
      rewritePacketBinding(packet, "address.json");
    }
    return packet;
  };
  const expectCorruptDiscoveryPacket = (name, mutate, expectedText) => {
    const packet = copyCompletedDiscoveryPacket(name);
    let result;
    try {
      mutate(packet);
      result = run(
        repo,
        stageArgs(
          base,
          base,
          `${name}-r1`,
          selectionPath,
          candidatePath,
          discoveryRequestPath,
        ),
      );
    } finally {
      rmSync(packet, { recursive: true, force: true });
    }
    check(
      `${name} discovery packet fails closed`,
      result.status === 2 &&
        /category=invalid-completion-packet count=1/.test(result.text) &&
        expectedText.test(result.text) &&
        !existsSync(join(repo, ".scratch", "panel", `${name}-r1`, ".complete")),
      result.text,
    );
  };
  const alternateDiscoveryDir = join(
    repo,
    ".scratch",
    "panel",
    "alternate-prefix-r1",
  );
  const firstPacketTemplateDir = join(repo, "first-packet-template");
  renameSync(firstDir, firstPacketTemplateDir);
  mkdirSync(alternateDiscoveryDir, { recursive: true });
  writeFileSync(
    join(alternateDiscoveryDir, ".complete"),
    savedFirstCompletionMarker,
  );
  chmodSync(join(alternateDiscoveryDir, ".complete"), 0o444);
  const markerOnlyDiscovery = run(
    repo,
    stageArgs(
      base,
      base,
      "otherprefix-r1",
      selectionPath,
      candidatePath,
      discoveryRequestPath,
    ),
  );
  check(
    "a marker-only packet is ignored as non-authoritative scratch",
    markerOnlyDiscovery.status === 0 &&
      existsSync(join(repo, ".scratch", "panel", "otherprefix-r1", ".complete")),
    markerOnlyDiscovery.text,
  );
  rmSync(alternateDiscoveryDir, { recursive: true, force: true });
  rmSync(join(repo, ".scratch", "panel", "otherprefix-r1"), {
    recursive: true,
    force: true,
  });
  const markerWithRemnantDir = join(
    repo,
    ".scratch",
    "panel",
    "marker-with-remnant",
  );
  mkdirSync(markerWithRemnantDir, { recursive: true });
  writeFileSync(
    join(markerWithRemnantDir, ".complete"),
    savedFirstCompletionMarker,
  );
  chmodSync(join(markerWithRemnantDir, ".complete"), 0o444);
  writeFileSync(join(markerWithRemnantDir, "responses.json"), "{}\n");
  const markerWithRemnantDiscovery = run(
    repo,
    stageArgs(
      base,
      base,
      "markerremnant-r1",
      selectionPath,
      candidatePath,
      discoveryRequestPath,
    ),
  );
  check(
    "a marker plus any packet remnant remains fail-closed",
    markerWithRemnantDiscovery.status === 2 &&
      /category=invalid-completion-packet count=1/.test(
        markerWithRemnantDiscovery.text,
      ) &&
      !existsSync(
        join(repo, ".scratch", "panel", "markerremnant-r1", ".complete"),
      ),
    markerWithRemnantDiscovery.text,
  );
  rmSync(markerWithRemnantDir, { recursive: true, force: true });
  const validDiscoveryDir = copyCompletedDiscoveryPacket(
    "valid-prefix-r1",
    true,
  );
  const validPrefixDiscovery = run(
    repo,
    stageArgs(
      base,
      base,
      "validprefix-r1",
      selectionPath,
      candidatePath,
      discoveryRequestPath,
    ),
  );
  check(
    "a fully validated discovery packet still blocks a second discovery",
    validPrefixDiscovery.status === 2 &&
      /category=completed-discovery count=1/.test(validPrefixDiscovery.text) &&
      !existsSync(
      join(repo, ".scratch", "panel", "validprefix-r1", ".complete"),
      ),
    validPrefixDiscovery.text,
  );
  rmSync(validDiscoveryDir, { recursive: true, force: true });

  expectCorruptDiscoveryPacket(
    "partialpacket",
    (packet) => rmSync(join(packet, "reviewer-notes", "software.md")),
    /category=invalid-completion-packet count=1/,
  );
  expectCorruptDiscoveryPacket(
    "deletedpacket",
    (packet) => rmSync(join(packet, "delta.diff")),
    /category=invalid-completion-packet count=1/,
  );
  expectCorruptDiscoveryPacket(
    "digestpacket",
    (packet) => {
      const evidence = join(packet, "evidence.md");
      chmodSync(evidence, 0o644);
      writeFileSync(evidence, "changed after completion\n");
    },
    /category=invalid-completion-packet count=1/,
  );
  expectCorruptDiscoveryPacket(
    "selectionpacket",
    (packet) => {
      const selectionFile = join(packet, "selection.json");
      const selection = readJson(selectionFile);
      selection.candidate_id = "e".repeat(64);
      chmodSync(selectionFile, 0o644);
      writeJson(selectionFile, selection);
      rewritePacketBinding(packet, "selection.json");
    },
    /category=invalid-completion-packet count=1/,
  );
  expectCorruptDiscoveryPacket(
    "addresspacket",
    (packet) => {
      const addressFile = join(packet, "address.json");
      const address = readJson(addressFile);
      address.tip = "0".repeat(40);
      chmodSync(addressFile, 0o644);
      writeJson(addressFile, address);
      rewritePacketBinding(packet, "address.json");
    },
    /category=invalid-completion-packet count=1/,
  );
  expectCorruptDiscoveryPacket(
    "deltapacket",
    (packet) => {
      const delta = join(packet, "delta.diff");
      chmodSync(delta, 0o644);
      writeFileSync(delta, "changed delta\n");
      rewritePacketBinding(packet, "delta.diff");
    },
    /category=invalid-completion-packet count=1/,
  );
  expectCorruptDiscoveryPacket(
    "fullpacket",
    (packet) => {
      const full = join(packet, "full.diff");
      chmodSync(full, 0o644);
      writeFileSync(full, "changed full\n");
      rewritePacketBinding(packet, "full.diff");
    },
    /category=invalid-completion-packet count=1/,
  );
  expectCorruptDiscoveryPacket(
    "tippacket",
    (packet) => {
      const markerPath = join(packet, ".complete");
      const marker = readJson(markerPath);
      marker.tip = "not-a-commit";
      chmodSync(markerPath, 0o644);
      writeJson(markerPath, marker);
      chmodSync(markerPath, 0o444);
    },
    /category=invalid-completion-packet count=1/,
  );

  const deletedMarkerPacket = copyCompletedDiscoveryPacket(
    "deletedmarker",
    true,
  );
  rmSync(join(deletedMarkerPacket, ".complete"));
  rmSync(join(deletedMarkerPacket, "delta.diff"));
  const deletedMarkerDiscovery = run(
    repo,
    stageArgs(
      base,
      base,
      "deletedmarker-r1",
      selectionPath,
      candidatePath,
      discoveryRequestPath,
    ),
  );
  check(
    "a deleted completion marker with remaining canonical artifacts blocks discovery",
    deletedMarkerDiscovery.status === 2 &&
      /category=canonical-remnant count=13/.test(
        deletedMarkerDiscovery.text,
      ) &&
      !existsSync(join(repo, ".scratch", "panel", "deletedmarker-r1", ".complete")),
    deletedMarkerDiscovery.text,
  );
  rmSync(deletedMarkerPacket, { recursive: true, force: true });

  const runUnmarkedPacketDiscovery = (name, populate) => {
    const packet = join(repo, ".scratch", "panel", name);
    const round = `unmarked${name.replace(/[^A-Za-z0-9]/g, "")}-r1`;
    mkdirSync(packet, { recursive: true });
    populate(packet);
    const result = run(
      repo,
      stageArgs(
        base,
        base,
        round,
        selectionPath,
        candidatePath,
        discoveryRequestPath,
      ),
    );
    rmSync(packet, { recursive: true, force: true });
    rmSync(join(repo, ".scratch", "panel", round), {
      recursive: true,
      force: true,
    });
    return result;
  };

  const soleCanonicalFile = runUnmarkedPacketDiscovery(
    "sole-canonical-file",
    (packet) => writeFileSync(join(packet, "address.json"), "{}\n"),
  );
  check(
    "a sole canonical file is a bounded packet remnant",
    soleCanonicalFile.status === 2 &&
      /category=canonical-remnant count=1/.test(
        soleCanonicalFile.text,
      ),
    soleCanonicalFile.text,
  );

  const nestedPacketArtifact = runUnmarkedPacketDiscovery(
    "nested-packet-artifact",
    (packet) => {
      const notes = join(packet, "reviewer-notes");
      mkdirSync(notes, { recursive: true });
      writeFileSync(join(notes, "software.md"), "nested artifact\n");
    },
  );
  check(
    "a nested artifact is reported only by its canonical top-level directory",
    nestedPacketArtifact.status === 2 &&
      /category=canonical-remnant count=1/.test(
        nestedPacketArtifact.text,
      ) &&
      !nestedPacketArtifact.text.includes("software.md"),
    nestedPacketArtifact.text,
  );

  const emptyCanonicalDirectory = runUnmarkedPacketDiscovery(
    "empty-canonical-directory",
    (packet) => mkdirSync(join(packet, "verification"), { recursive: true }),
  );
  check(
    "an empty canonical directory is a packet remnant",
    emptyCanonicalDirectory.status === 2 &&
      /category=canonical-remnant count=1/.test(
        emptyCanonicalDirectory.text,
      ),
    emptyCanonicalDirectory.text,
  );

  for (const [name, otherName] of [
    ["lone-ledger", "discovery-ledger.json"],
    ["lone-responses", "responses.json"],
  ]) {
    const loneHandoff = runUnmarkedPacketDiscovery(name, (packet) =>
      writeFileSync(join(packet, otherName), "{}\n"),
    );
    check(
      `${otherName} without its handoff pair blocks discovery`,
      loneHandoff.status === 2 &&
        /category=partial-handoff count=1/.test(loneHandoff.text),
      loneHandoff.text,
    );
  }

  const nestedNamesPacket = runUnmarkedPacketDiscovery(
    "deep-wide-sensitive-names",
    (packet) => {
      let deep = join(packet, "agent-definitions");
      mkdirSync(deep, { recursive: true });
      for (let depth = 0; depth < 24; depth += 1) {
        deep = join(deep, `deep-${depth}`);
        mkdirSync(deep, { recursive: true });
      }
      writeFileSync(
        join(deep, "sensitive-input-derived-secret-token.txt"),
        "sensitive nested name\n",
      );
      const wide = join(packet, "reviewer-notes");
      mkdirSync(wide, { recursive: true });
      for (let index = 0; index < 256; index += 1) {
        writeFileSync(
          join(wide, `wide-sensitive-input-${index}.md`),
          "nested artifact\n",
        );
      }
    },
  );
  check(
    "deep and wide nested names never enter packet diagnostics",
    nestedNamesPacket.status === 2 &&
      /category=canonical-remnant count=2/.test(
        nestedNamesPacket.text,
      ) &&
      !nestedNamesPacket.text.includes("sensitive-input-derived-secret-token") &&
      !nestedNamesPacket.text.includes("wide-sensitive-input-"),
    nestedNamesPacket.text,
  );
  check(
    "packet remnant diagnostics remain bounded",
    nestedNamesPacket.text.length < 2000 &&
      !nestedNamesPacket.text.includes("deep-0") &&
      !nestedNamesPacket.text.includes("wide-sensitive-input-"),
    nestedNamesPacket.text,
  );

  const emptyPacket = join(repo, ".scratch", "panel", "empty-packet");
  mkdirSync(emptyPacket, { recursive: true });
  const emptyPacketDiscovery = run(
    repo,
    stageArgs(
      base,
      base,
      "emptypacket-r1",
      selectionPath,
      candidatePath,
      discoveryRequestPath,
    ),
  );
  check(
    "an empty unmarked packet does not block discovery",
    emptyPacketDiscovery.status === 0 &&
      existsSync(join(repo, ".scratch", "panel", "emptypacket-r1", ".complete")),
    emptyPacketDiscovery.text,
  );
  rmSync(emptyPacket, { recursive: true, force: true });
  rmSync(join(repo, ".scratch", "panel", "emptypacket-r1"), {
    recursive: true,
    force: true,
  });

  const irrelevantPacket = join(
    repo,
    ".scratch",
    "panel",
    "irrelevant-packet",
  );
  mkdirSync(irrelevantPacket, { recursive: true });
  writeFileSync(join(irrelevantPacket, "operator-note.txt"), "not a packet\n");
  const irrelevantPacketDiscovery = run(
    repo,
    stageArgs(
      base,
      base,
      "irrelevantpacket-r1",
      selectionPath,
      candidatePath,
      discoveryRequestPath,
    ),
  );
  check(
    "an unmarked packet with only irrelevant files does not block discovery",
    irrelevantPacketDiscovery.status === 0 &&
      existsSync(
        join(repo, ".scratch", "panel", "irrelevantpacket-r1", ".complete"),
      ),
    irrelevantPacketDiscovery.text,
  );
  rmSync(irrelevantPacket, { recursive: true, force: true });
  rmSync(join(repo, ".scratch", "panel", "irrelevantpacket-r1"), {
    recursive: true,
    force: true,
  });

  renameSync(firstPacketTemplateDir, firstDir);
  writeFileSync(join(firstDir, ".complete"), savedFirstCompletionMarker);
  chmodSync(join(firstDir, ".complete"), 0o444);

  const reusedSecondDiscovery = run(
    repo,
    stageArgs(
      base,
      firstTip,
      "spec001w1-r2",
      selectionPath,
      candidatePath,
      discoveryRequestPath,
    ),
  );
  check(
    "a completed discovery packet requires verification for the next round",
    reusedSecondDiscovery.status === 2 &&
      /requires a verification selection|must not run a second discovery/.test(
        reusedSecondDiscovery.text,
      ) &&
      !existsSync(join(repo, ".scratch", "panel", "spec001w1-r2", ".complete")),
    reusedSecondDiscovery.text,
  );

  const savedDiscoveryRequest = readFileSync(
    join(firstDir, "discovery-request.json"),
  );
  rmSync(join(firstDir, "discovery-request.json"));
  const completeMissingCanonical = run(
    repo,
    stageArgs(base, base, "spec001w1-r1", selectionPath, candidatePath, discoveryRequestPath),
  );
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

  const stagedEvidencePath = join(firstDir, "evidence.md");
  const originalEvidenceBytes = readFileSync(stagedEvidencePath);
  chmodSync(stagedEvidencePath, 0o644);
  writeFileSync(stagedEvidencePath, "mutated evidence\n");
  const mutatedEvidence = run(
    repo,
    stageArgs(base, base, "spec001w1-r1", selectionPath, candidatePath, discoveryRequestPath),
  );
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

  for (const seat of firstRoster) {
    writeJson(join(firstDir, "verdicts", `${seat}.json`), {
      engineer: seat,
      signoff: true,
      summary: "Reviewed.",
      recommendations: [],
    });
  }

  writeFileSync(join(repo, "Makefile"), "second build change\n");
  git(repo, "add", "Makefile");
  git(repo, "commit", "--quiet", "-m", "second");
  const secondTip = git(repo, "rev-parse", "HEAD");
  const currentCandidatePath = join(repo, "current-candidate.json");
  writeJson(currentCandidatePath, {
    program: "SPEC001",
    wave: "spec001w1",
    candidate_id: "d".repeat(64),
    content_id: "e".repeat(64),
    snapshot_sha256: "f".repeat(64),
    changed_paths: ["Makefile", "first.txt", literalBackslashPath],
  });
  const deltaPath = join(repo, "fix-delta.json");
  writeJson(deltaPath, { changed_paths: ["Makefile"] });
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
  const stagedLedger = join(repo, "discovery-ledger-source.json");
  const stagedResponses = join(repo, "responses-source.json");
  const stagedSelfVerification = join(repo, "self-verification-source.json");
  const discoveryResultsPath = join(repo, "discovery-results.json");
  const groupsPath = join(repo, "discovery-groups.json");
  const firstVerdict = readJson(join(firstDir, "verdicts", "software.json"));
  firstVerdict.signoff = false;
  firstVerdict.summary = "The fix needs verification.";
  firstVerdict.recommendations = [{
    severity: "high",
    where: "Makefile",
    what: "The fix needs verification.",
    why: "The change could regress the handoff.",
    fix: "Verify the fix.",
  }];
  writeJson(join(firstDir, "verdicts", "software.json"), firstVerdict);
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
  writeJson(groupsPath, [{
    source_finding_ids: ["software:1"],
    description: "The fix needs verification.",
    severity: "MAJOR",
    impact: "The change could regress the handoff.",
    recommendation: "Verify the fix.",
  }]);
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
  const responseEnvelope = readJson(stagedResponses);
  responseEnvelope.responses[0] = {
    issue_id: "R1",
    disposition: "Fixed",
    changed_surface: ["Makefile"],
    justification: "The fix is staged for verification.",
    evidence: "focused test",
  };
  writeJson(stagedResponses, responseEnvelope);
  writeJson(stagedSelfVerification, {
    tests: ["focused stage test"],
    lint: "passed",
    formatting: "passed",
    static_analysis: "passed",
    build: "not applicable",
    uncovered_areas: ["none"],
    self_review: "passed",
  });
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
  const verificationRoster = readJson(currentSelectionPath).roster;

  const continuationSelection = readJson(currentSelectionPath);
  const continuationLedger = readJson(stagedLedger);
  const continuationResponses = readJson(stagedResponses);
  const continuationVerification = createVerificationResultArtifact({
    selection: continuationSelection,
    selection_bytes: readFileSync(currentSelectionPath, "utf8"),
    ledger: continuationLedger,
    ledger_bytes: readFileSync(stagedLedger, "utf8"),
    current_candidate: readJson(currentCandidatePath),
    results: Object.fromEntries(
      verificationRoster.map((seat) => [
        seat,
        {
          seat,
          complete: true,
          signoff: true,
          summary: "Verified.",
          recommendations: [],
          verified_issue_statuses: Object.fromEntries(
            continuationLedger.issues.map((issue) => [issue.id, "verified"]),
          ),
          late_findings: [],
        },
      ]),
    ),
  });
  const realHandoffDir = join(repo, "real-advance-handoff");
  const realHandoffInput = {
    current_selection: continuationSelection,
    selection_bytes: readFileSync(currentSelectionPath, "utf8"),
    discovery_ledger: continuationLedger,
    discovery_ledger_bytes: readFileSync(stagedLedger, "utf8"),
    responses: continuationResponses,
    responses_bytes: readFileSync(stagedResponses, "utf8"),
    verification_results: continuationVerification,
    verification_results_bytes: stableStringify(continuationVerification),
    current_candidate: readJson(currentCandidatePath),
  };
  const firstHandoffPublication = writeAdvanceVerification(
    realHandoffDir,
    realHandoffInput,
  );
  const secondHandoffPublication = writeAdvanceVerification(
    realHandoffDir,
    realHandoffInput,
  );
  const continuationLedgerPath = join(
    realHandoffDir,
    "discovery-ledger.json",
  );
  const continuationBlankResponsesPath = join(realHandoffDir, "responses.json");
  const continuationCompletedResponsesPath = join(
    realHandoffDir,
    "responses-completed.json",
  );
  cpSync(continuationBlankResponsesPath, continuationCompletedResponsesPath);
  const continuationCompletedResponses = readJson(
    continuationCompletedResponsesPath,
  );
  continuationCompletedResponses.responses = continuationCompletedResponses.responses.map(
    (response) => ({
      ...response,
      disposition: "Fixed",
      changed_surface: ["Makefile"],
      justification: "The continuation response is complete.",
      evidence: "focused continuation test",
    }),
  );
  writeJson(
    continuationCompletedResponsesPath,
    continuationCompletedResponses,
  );
  const continuationHandoffPath = join(realHandoffDir, "handoff.json");
  const finalizedContinuation = writeFinalizeHandoff(continuationHandoffPath, {
    discovery_ledger: readJson(continuationLedgerPath),
    discovery_ledger_bytes: readFileSync(continuationLedgerPath, "utf8"),
    completed_responses: continuationCompletedResponses,
    completed_responses_bytes: readFileSync(
      continuationCompletedResponsesPath,
      "utf8",
    ),
  });
  check(
    "advance output publishes two files and finalization publishes the marker last",
    firstHandoffPublication.publication.ledger.created === true &&
      firstHandoffPublication.publication.responses.created === true &&
      secondHandoffPublication.publication.ledger.created === false &&
      secondHandoffPublication.publication.responses.created === false &&
      finalizedContinuation.publication.created === true &&
      readdirSync(realHandoffDir).sort().join(",") ===
        "discovery-ledger.json,handoff.json,responses-completed.json,responses.json",
  );
  check(
    "finalized continuation retry compares the marker",
    writeFinalizeHandoff(continuationHandoffPath, {
      discovery_ledger: readJson(continuationLedgerPath),
      discovery_ledger_bytes: readFileSync(continuationLedgerPath, "utf8"),
      completed_responses: continuationCompletedResponses,
      completed_responses_bytes: readFileSync(
        continuationCompletedResponsesPath,
        "utf8",
      ),
    }).publication.created === false &&
      secondHandoffPublication.publication.ledger.created === false &&
      secondHandoffPublication.publication.responses.created === false,
  );
  const continuationVerificationDir = join(
    repo,
    "continuation-verification-requests",
  );
  const continuationVerificationProcess = spawnSync("node", [
    lifecycleScript,
    "verification",
    currentSelectionPath,
    continuationLedgerPath,
    continuationCompletedResponsesPath,
    stagedSelfVerification,
    continuationVerificationDir,
    "--candidate",
    currentCandidatePath,
    "--prior-selection",
    selectionPath,
    "--prior-verdicts",
    join(firstDir, "verdicts"),
    "--delta",
    deltaPath,
    "--handoff",
    continuationHandoffPath,
  ], { cwd: repo, encoding: "utf8" });
  const continuationVerificationPreparation = {
    status: continuationVerificationProcess.status,
    text: `${continuationVerificationProcess.stdout || ""}${continuationVerificationProcess.stderr || ""}`,
  };
  check(
    "verification preparation validates and consumes the explicit handoff",
    continuationVerificationPreparation.status === 0 &&
      existsSync(join(continuationVerificationDir, "software.json")),
    continuationVerificationPreparation.text,
  );

  const unrelatedCandidatePath = join(repo, "unrelated-candidate.json");
  const unrelatedSelectionPath = join(repo, "unrelated-selection.json");
  const unrelatedRequestPath = join(repo, "unrelated-request.json");
  writeJson(unrelatedCandidatePath, {
    program: "SPEC002",
    wave: "spec002w1",
    candidate_id: "1".repeat(64),
    content_id: "2".repeat(64),
    snapshot_sha256: "3".repeat(64),
    changed_paths: ["Makefile", "first.txt", literalBackslashPath],
  });
  const generatedUnrelatedSelection = execFileSync(
    "node",
    [
      lifecycleScript,
      "select",
      unrelatedCandidatePath,
      "spec002w1",
      "--git-range",
      `${base}..${secondTip}`,
    ],
    { cwd: repo, encoding: "utf8" },
  ).trim();
  cpSync(generatedUnrelatedSelection, unrelatedSelectionPath);
  execFileSync(
    "node",
    [
      lifecycleScript,
      "discovery-request",
      unrelatedSelectionPath,
      unrelatedCandidatePath,
      unrelatedRequestPath,
    ],
    { cwd: repo, encoding: "utf8" },
  );

  const runRealHandoffDiscovery = (
    name,
    selection,
    candidate,
    request,
    lifecycle,
    mutate,
    options = {},
  ) => {
    const packet = join(repo, ".scratch", "panel", name);
    const round = `${name.replace(/[^A-Za-z0-9]/g, "")}-r1`;
    cpSync(realHandoffDir, packet, { recursive: true });
    try {
      if (mutate) mutate(packet);
      return run(
        repo,
        stageArgs(base, base, round, selection, candidate, request, lifecycle),
        options,
      );
    } finally {
      rmSync(packet, { recursive: true, force: true });
      rmSync(join(repo, ".scratch", "panel", round), {
        recursive: true,
        force: true,
      });
    }
  };

  const unrelatedHandoff = runRealHandoffDiscovery(
    "handoff-with-unrelated-lifecycle-and-sensitive-name",
    unrelatedSelectionPath,
    unrelatedCandidatePath,
    unrelatedRequestPath,
    "spec002w1",
  );
  check(
    "an unrelated valid handoff is ignored",
    unrelatedHandoff.status === 0 &&
      !unrelatedHandoff.text.includes(
        "handoff-with-unrelated-lifecycle-and-sensitive-name",
      ),
    unrelatedHandoff.text,
  );
  const retainedContinuationOutputs = runRealHandoffDiscovery(
    "retained-continuation-outputs",
    unrelatedSelectionPath,
    unrelatedCandidatePath,
    unrelatedRequestPath,
    "spec002w1",
    (packet) => {
      mkdirSync(join(packet, "verification"), { recursive: true });
      writeFileSync(
        join(packet, "verification", "software.json"),
        "{}\n",
      );
      writeFileSync(join(packet, "self-verification.json"), "{}\n");
      writeFileSync(join(packet, "candidate.json"), "{}\n");
      writeFileSync(join(packet, "fix-delta.json"), "{}\n");
      writeFileSync(join(packet, "evidence.md"), "post-finalization evidence\n");
    },
  );
  check(
    "discovery scan permits documented retained continuation outputs",
    retainedContinuationOutputs.status === 0,
    retainedContinuationOutputs.text,
  );
  const stagedContinuationAnchors = runRealHandoffDiscovery(
    "staged-continuation-anchors",
    unrelatedSelectionPath,
    unrelatedCandidatePath,
    unrelatedRequestPath,
    "spec002w1",
    (packet) => {
      writeFileSync(join(packet, "selection.json"), "{}\n");
      writeFileSync(join(packet, "address.json"), "{}\n");
      writeFileSync(join(packet, "full.diff"), "staged anchor\n");
    },
  );
  check(
    "discovery scan rejects staged-packet anchors in a retained handoff",
    stagedContinuationAnchors.status === 2 &&
      /category=damaged-handoff count=3/.test(
        stagedContinuationAnchors.text,
      ) &&
      !stagedContinuationAnchors.text.includes(
        "staged-continuation-anchors",
      ),
    stagedContinuationAnchors.text,
  );

  const discoveryPacketBackup = join(repo, "discovery-packet-backup");
  cpSync(firstDir, discoveryPacketBackup, { recursive: true });
  rmSync(firstDir, { recursive: true, force: true });
  const sameLifecycleHandoff = runRealHandoffDiscovery(
    "same-lifecycle-handoff",
    selectionPath,
    candidatePath,
    discoveryRequestPath,
    "spec001w1",
  );
  cpSync(discoveryPacketBackup, firstDir, { recursive: true });
  rmSync(discoveryPacketBackup, { recursive: true, force: true });
  check(
    "a same-lifecycle handoff blocks discovery",
    sameLifecycleHandoff.status === 2 &&
      /category=same-lifecycle-handoff count=1/.test(sameLifecycleHandoff.text),
    sameLifecycleHandoff.text,
  );

  const unmarkedPair = runRealHandoffDiscovery(
    "unmarked-ledger-response-pair",
    unrelatedSelectionPath,
    unrelatedCandidatePath,
    unrelatedRequestPath,
    "spec002w1",
    (packet) => {
      rmSync(join(packet, "handoff.json"));
      rmSync(join(packet, "responses-completed.json"));
    },
  );
  check(
    "an unmarked ledger-response pair is damaged packet state",
    unmarkedPair.status === 2 &&
      /category=damaged-handoff count=2/.test(unmarkedPair.text),
    unmarkedPair.text,
  );

  const missingFinalizedMarker = runRealHandoffDiscovery(
    "missing-finalized-marker",
    unrelatedSelectionPath,
    unrelatedCandidatePath,
    unrelatedRequestPath,
    "spec002w1",
    (packet) => rmSync(join(packet, "handoff.json")),
  );
  check(
    "a completed response pair without its finalized marker fails closed",
    missingFinalizedMarker.status === 2 &&
      /category=partial-handoff count=3/.test(missingFinalizedMarker.text),
    missingFinalizedMarker.text,
  );

  const malformedMarker = runRealHandoffDiscovery(
    "malformed-sensitive-handoff-marker",
    unrelatedSelectionPath,
    unrelatedCandidatePath,
    unrelatedRequestPath,
    "spec002w1",
    (packet) => writeFileSync(join(packet, "handoff.json"), "{not-json\n"),
  );
  check(
    "a malformed handoff marker fails closed without its packet name",
    malformedMarker.status === 2 &&
      /category=invalid-handoff count=1/.test(malformedMarker.text) &&
      !malformedMarker.text.includes("malformed-sensitive-handoff-marker"),
    malformedMarker.text,
  );

  const digestMismatch = runRealHandoffDiscovery(
    "handoff-digest-mismatch",
    unrelatedSelectionPath,
    unrelatedCandidatePath,
    unrelatedRequestPath,
    "spec002w1",
    (packet) => {
      const markerPath = join(packet, "handoff.json");
      const marker = readJson(markerPath);
      marker.ledger_sha256 = "f".repeat(64);
      writeJson(markerPath, marker);
    },
  );
  check(
    "a handoff digest mismatch fails closed",
    digestMismatch.status === 2 &&
      /category=invalid-handoff count=1/.test(digestMismatch.text),
    digestMismatch.text,
  );

  const sizeMismatch = runRealHandoffDiscovery(
    "handoff-size-mismatch",
    unrelatedSelectionPath,
    unrelatedCandidatePath,
    unrelatedRequestPath,
    "spec002w1",
    (packet) => {
      const markerPath = join(packet, "handoff.json");
      const marker = readJson(markerPath);
      marker.responses_bytes += 1;
      writeJson(markerPath, marker);
    },
  );
  check(
    "a handoff byte-size mismatch fails closed",
    sizeMismatch.status === 2 &&
      /category=invalid-handoff count=1/.test(sizeMismatch.text),
    sizeMismatch.text,
  );

  const envelopeMismatch = runRealHandoffDiscovery(
    "handoff-envelope-mismatch",
    unrelatedSelectionPath,
    unrelatedCandidatePath,
    unrelatedRequestPath,
    "spec002w1",
    (packet) => {
      const markerPath = join(packet, "handoff.json");
      const marker = readJson(markerPath);
      marker.candidate_id = "e".repeat(64);
      writeJson(markerPath, marker);
    },
  );
  check(
    "a handoff envelope mismatch fails closed",
    envelopeMismatch.status === 2 &&
      /category=invalid-handoff count=1/.test(envelopeMismatch.text),
    envelopeMismatch.text,
  );

  const partialHandoff = runRealHandoffDiscovery(
    "partial-handoff-files",
    unrelatedSelectionPath,
    unrelatedCandidatePath,
    unrelatedRequestPath,
    "spec002w1",
    (packet) => {
      rmSync(join(packet, "responses.json"));
      rmSync(join(packet, "responses-completed.json"));
    },
  );
  check(
    "partial handoff files fail closed",
    partialHandoff.status === 2 &&
      /category=partial-handoff count=2/.test(partialHandoff.text),
    partialHandoff.text,
  );

  const overLimitPacket = join(
    repo,
    ".scratch",
    "panel",
    "over-limit-sensitive-packet-name",
  );
  mkdirSync(overLimitPacket, { recursive: true });
  for (let index = 0; index <= 4096; index += 1) {
    writeFileSync(join(overLimitPacket, `entry-${index}.tmp`), "");
  }
  const overLimitDiscovery = run(
    repo,
    stageArgs(
      base,
      base,
      "overlimit-r1",
      unrelatedSelectionPath,
      unrelatedCandidatePath,
      unrelatedRequestPath,
      "spec002w1",
    ),
  );
  check(
    "top-level packet entry limits fail early with redacted diagnostics",
    overLimitDiscovery.status === 2 &&
      /category=top-level-entry-limit count=4097/.test(
        overLimitDiscovery.text,
      ) &&
      !overLimitDiscovery.text.includes("over-limit-sensitive-packet-name"),
    overLimitDiscovery.text,
  );
  rmSync(overLimitPacket, { recursive: true, force: true });

  const dtUnknownShim = join(repo, "dt-unknown.cjs");
  writeFileSync(
    dtUnknownShim,
    "const { Dirent } = require('node:fs'); Dirent.prototype.isFile = () => false; Dirent.prototype.isDirectory = () => false;\n",
  );
  const dtUnknownDiscovery = runRealHandoffDiscovery(
    "dt-unknown-fallback",
    unrelatedSelectionPath,
    unrelatedCandidatePath,
    unrelatedRequestPath,
    "spec002w1",
    undefined,
    { env: { NODE_OPTIONS: `--require ${dtUnknownShim}` } },
  );
  check(
    "DT_UNKNOWN simulation uses lstat fallback for relevant entries",
    dtUnknownDiscovery.status === 0,
    dtUnknownDiscovery.text,
  );

  console.log("stage-diffs: later review");
  const predecessorMarker = join(firstDir, ".complete");
  const predecessorBytes = readFileSync(predecessorMarker);
  const predecessorDispatchBinding = readFileSync(
    join(firstDir, "dispatch-binding.json"),
  );
  const predecessorDefinitions = Object.fromEntries(
    firstRoster.map((seat) => [
      seat,
      readFileSync(
        join(firstDir, "agent-definitions", `panel-${seat}.agent.md`),
      ),
    ]),
  );
  const restorePredecessorMarker = () => {
    writeFileSync(predecessorMarker, predecessorBytes);
    chmodSync(predecessorMarker, 0o444);
  };
  const verificationStage = (previousTip = firstTip) =>
    run(repo, [
      base,
      previousTip,
      "spec001w1-r2",
      "--selection",
      currentSelectionPath,
      "--candidate",
      currentCandidatePath,
      "--ledger",
      continuationLedgerPath,
      "--responses",
      continuationCompletedResponsesPath,
      "--handoff",
      continuationHandoffPath,
      "--self-verification",
      stagedSelfVerification,
      "--verification-dir",
      continuationVerificationDir,
    ]);
  rmSync(predecessorMarker);
  const missingPredecessor = run(repo, [
    base,
    firstTip,
    "spec001w1-r2",
    "--selection",
    currentSelectionPath,
    "--candidate",
    currentCandidatePath,
    "--ledger",
    continuationLedgerPath,
    "--responses",
    continuationCompletedResponsesPath,
    "--handoff",
    continuationHandoffPath,
    "--self-verification",
    stagedSelfVerification,
    "--verification-dir",
    continuationVerificationDir,
  ]);
  check(
    "later staging requires a canonical predecessor marker first",
    missingPredecessor.status === 2 &&
      /missing canonical predecessor packet/.test(missingPredecessor.text),
    missingPredecessor.text,
  );
  restorePredecessorMarker();

  const firstVerificationWithoutHandoff = run(repo, [
    base,
    firstTip,
    "spec001w1-r2",
    "--selection",
    currentSelectionPath,
    "--candidate",
    currentCandidatePath,
    "--ledger",
    continuationLedgerPath,
    "--responses",
    continuationCompletedResponsesPath,
    "--self-verification",
    stagedSelfVerification,
    "--verification-dir",
    continuationVerificationDir,
  ]);
  const firstVerificationPacket = join(
    repo,
    ".scratch",
    "panel",
    "spec001w1-r2",
  );
  check(
    "discovery-to-first-verification staging remains marker-free",
    firstVerificationWithoutHandoff.status === 0 &&
      !existsSync(join(firstVerificationPacket, "handoff.json")) &&
      existsSync(join(firstVerificationPacket, ".complete")),
    firstVerificationWithoutHandoff.text,
  );
  rmSync(firstVerificationPacket, { recursive: true, force: true });

  const originalContinuationHandoffBytes = readFileSync(continuationHandoffPath);
  const invalidContinuationHandoff = readJson(continuationHandoffPath);
  invalidContinuationHandoff.responses_sha256 = "f".repeat(64);
  chmodSync(continuationHandoffPath, 0o644);
  writeJson(continuationHandoffPath, invalidContinuationHandoff);
  const invalidHandoffStage = verificationStage();
  check(
    "verification staging validates handoff-bound bytes before publication",
    invalidHandoffStage.status === 2 &&
      /does not bind the exact ledger and completed responses/.test(
        invalidHandoffStage.text,
      ) &&
      !existsSync(join(repo, ".scratch", "panel", "spec001w1-r2", ".complete")),
    invalidHandoffStage.text,
  );
  writeFileSync(continuationHandoffPath, originalContinuationHandoffBytes);
  chmodSync(continuationHandoffPath, 0o444);

  const schema2Marker = readJson(predecessorMarker);
  for (const seat of firstRoster) {
    delete schema2Marker.artifact_sha256[
      `agent-definitions/panel-${seat}.agent.md`
    ];
    delete schema2Marker.artifact_bytes[
      `agent-definitions/panel-${seat}.agent.md`
    ];
  }
  delete schema2Marker.artifact_sha256["dispatch-binding.json"];
  delete schema2Marker.artifact_bytes["dispatch-binding.json"];
  schema2Marker.schema_version = 2;
  chmodSync(predecessorMarker, 0o644);
  writeJson(predecessorMarker, schema2Marker);
  rmSync(join(firstDir, "dispatch-binding.json"));
  rmSync(join(firstDir, "agent-definitions"), { recursive: true, force: true });
  const schema2Predecessor = verificationStage();
  const schema2OutputDir = join(repo, ".scratch", "panel", "spec001w1-r2");
  check(
    "schema-2 predecessor without definitions upgrades to a schema-4 packet",
    schema2Predecessor.status === 0 &&
      readJson(join(schema2OutputDir, ".complete")).schema_version === 4 &&
      existsSync(join(schema2OutputDir, "dispatch-binding.json")),
    schema2Predecessor.text,
  );
  rmSync(schema2OutputDir, { recursive: true, force: true });
  writeFileSync(
    join(firstDir, "dispatch-binding.json"),
    predecessorDispatchBinding,
  );
  mkdirSync(join(firstDir, "agent-definitions"), { recursive: true });
  for (const [seat, bytes] of Object.entries(predecessorDefinitions)) {
    writeFileSync(
      join(firstDir, "agent-definitions", `panel-${seat}.agent.md`),
      bytes,
    );
  }
  restorePredecessorMarker();

  const schema2WithDefinitionsMarker = readJson(predecessorMarker);
  delete schema2WithDefinitionsMarker.artifact_sha256["dispatch-binding.json"];
  delete schema2WithDefinitionsMarker.artifact_bytes["dispatch-binding.json"];
  schema2WithDefinitionsMarker.schema_version = 2;
  chmodSync(predecessorMarker, 0o644);
  writeJson(predecessorMarker, schema2WithDefinitionsMarker);
  rmSync(join(firstDir, "dispatch-binding.json"));
  const schema2WithDefinitions = verificationStage();
  check(
    "schema-2 predecessor with definitions and without dispatch binding upgrades",
    schema2WithDefinitions.status === 0 &&
      readJson(join(schema2OutputDir, ".complete")).schema_version === 4 &&
      existsSync(join(schema2OutputDir, "dispatch-binding.json")),
    schema2WithDefinitions.text,
  );
  rmSync(schema2OutputDir, { recursive: true, force: true });
  writeFileSync(
    join(firstDir, "dispatch-binding.json"),
    predecessorDispatchBinding,
  );
  restorePredecessorMarker();

  const schema2PartialDefinitions = readJson(predecessorMarker);
  delete schema2PartialDefinitions.artifact_sha256[
    `agent-definitions/panel-${firstRoster[0]}.agent.md`
  ];
  delete schema2PartialDefinitions.artifact_bytes[
    `agent-definitions/panel-${firstRoster[0]}.agent.md`
  ];
  delete schema2PartialDefinitions.artifact_sha256["dispatch-binding.json"];
  delete schema2PartialDefinitions.artifact_bytes["dispatch-binding.json"];
  schema2PartialDefinitions.schema_version = 2;
  chmodSync(predecessorMarker, 0o644);
  writeJson(predecessorMarker, schema2PartialDefinitions);
  const partialSchema2 = verificationStage();
  check(
    "schema-2 partial definitions are rejected as an arbitrary subset",
    partialSchema2.status === 2 &&
      /schema_version 2 requires exactly|completion artifact set is invalid/.test(
        partialSchema2.text,
      ) &&
      !existsSync(join(schema2OutputDir, ".complete")),
    partialSchema2.text,
  );
  restorePredecessorMarker();

  const schema3Marker = readJson(predecessorMarker);
  delete schema3Marker.artifact_sha256["dispatch-binding.json"];
  delete schema3Marker.artifact_bytes["dispatch-binding.json"];
  schema3Marker.schema_version = 3;
  chmodSync(predecessorMarker, 0o644);
  writeJson(predecessorMarker, schema3Marker);
  rmSync(join(firstDir, "dispatch-binding.json"));
  const schema3Predecessor = verificationStage();
  check(
    "schema-3 predecessor with definitions and without dispatch binding upgrades",
    schema3Predecessor.status === 0 &&
      readJson(join(schema2OutputDir, ".complete")).schema_version === 4 &&
      existsSync(join(schema2OutputDir, "dispatch-binding.json")),
    schema3Predecessor.text,
  );
  rmSync(schema2OutputDir, { recursive: true, force: true });
  writeFileSync(
    join(firstDir, "dispatch-binding.json"),
    predecessorDispatchBinding,
  );
  restorePredecessorMarker();

  const schema3MissingDefinition = readJson(predecessorMarker);
  delete schema3MissingDefinition.artifact_sha256[
    `agent-definitions/panel-${firstRoster[0]}.agent.md`
  ];
  delete schema3MissingDefinition.artifact_bytes[
    `agent-definitions/panel-${firstRoster[0]}.agent.md`
  ];
  schema3MissingDefinition.schema_version = 3;
  delete schema3MissingDefinition.artifact_sha256["dispatch-binding.json"];
  delete schema3MissingDefinition.artifact_bytes["dispatch-binding.json"];
  chmodSync(predecessorMarker, 0o644);
  writeJson(predecessorMarker, schema3MissingDefinition);
  rmSync(join(firstDir, "dispatch-binding.json"));
  const missingSchema3Definition = verificationStage();
  check(
    "schema-3 missing definition is rejected as an arbitrary subset",
    missingSchema3Definition.status === 2 &&
      /schema_version 3 requires exactly|completion artifact set is invalid/.test(
        missingSchema3Definition.text,
      ) &&
      !existsSync(join(schema2OutputDir, ".complete")),
    missingSchema3Definition.text,
  );
  writeFileSync(
    join(firstDir, "dispatch-binding.json"),
    predecessorDispatchBinding,
  );
  restorePredecessorMarker();

  const legacyMarker = readJson(predecessorMarker);
  legacyMarker.schema_version = 1;
  chmodSync(predecessorMarker, 0o644);
  writeJson(predecessorMarker, legacyMarker);
  const legacyPredecessor = verificationStage();
  check(
    "later staging rejects a legacy completion marker",
    legacyPredecessor.status === 2 &&
      /not a supported canonical completion packet/.test(
        legacyPredecessor.text,
      ) &&
      !existsSync(join(repo, ".scratch", "panel", "spec001w1-r2", ".complete")),
    legacyPredecessor.text,
  );
  restorePredecessorMarker();

  chmodSync(predecessorMarker, 0o644);
  writeFileSync(predecessorMarker, "{not-json\n");
  const corruptPredecessor = verificationStage();
  check(
    "later staging rejects a corrupt completion marker",
    corruptPredecessor.status === 2 &&
      /invalid completion marker/.test(corruptPredecessor.text) &&
      !existsSync(join(repo, ".scratch", "panel", "spec001w1-r2", ".complete")),
    corruptPredecessor.text,
  );
  restorePredecessorMarker();

  const omittedArtifactMarker = readJson(predecessorMarker);
  delete omittedArtifactMarker.artifact_sha256["evidence.md"];
  delete omittedArtifactMarker.artifact_bytes["evidence.md"];
  chmodSync(predecessorMarker, 0o644);
  writeJson(predecessorMarker, omittedArtifactMarker);
  const omittedArtifact = verificationStage();
  check(
    "later staging rejects an omitted completion artifact entry",
    omittedArtifact.status === 2 &&
      /completion artifact set is invalid.*missing/.test(omittedArtifact.text) &&
      !existsSync(join(repo, ".scratch", "panel", "spec001w1-r2", ".complete")),
    omittedArtifact.text,
  );
  restorePredecessorMarker();

  const extraArtifactMarker = readJson(predecessorMarker);
  extraArtifactMarker.artifact_sha256["extra.txt"] = "0".repeat(64);
  extraArtifactMarker.artifact_bytes["extra.txt"] = 0;
  chmodSync(predecessorMarker, 0o644);
  writeJson(predecessorMarker, extraArtifactMarker);
  const extraArtifact = verificationStage();
  check(
    "later staging rejects an extra completion artifact entry",
    extraArtifact.status === 2 &&
      /completion artifact set is invalid.*extra/.test(extraArtifact.text) &&
      !existsSync(join(repo, ".scratch", "panel", "spec001w1-r2", ".complete")),
    extraArtifact.text,
  );
  restorePredecessorMarker();

  const mutatedMarker = readJson(predecessorMarker);
  mutatedMarker.artifact_sha256["evidence.md"] = "0".repeat(64);
  chmodSync(predecessorMarker, 0o644);
  writeJson(predecessorMarker, mutatedMarker);
  const mutatedPredecessor = verificationStage();
  check(
    "later staging rejects a mutated completion marker binding",
    mutatedPredecessor.status === 2 &&
      /post-completion mutation of evidence\.md is refused/.test(
        mutatedPredecessor.text,
      ) &&
      !existsSync(join(repo, ".scratch", "panel", "spec001w1-r2", ".complete")),
    mutatedPredecessor.text,
  );
  restorePredecessorMarker();

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
      /missing previous verdict for incumbent seat test/.test(
        incompletePreviousReview.text,
      ),
    incompletePreviousReview.text,
  );
  writeFileSync(missingVerdict, savedVerdict);

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
    "verification staging requires every seat request",
    missingVerificationDirectory.status === 2 &&
      /--verification-dir is required/.test(missingVerificationDirectory.text),
    missingVerificationDirectory.text,
  );

  const removedVerificationSeat = verificationRoster[0];
  const removedVerificationRequest = join(
    continuationVerificationDir,
    `${removedVerificationSeat}.json`,
  );
  const removedVerificationBytes = readFileSync(removedVerificationRequest);
  rmSync(removedVerificationRequest);
  const missingOneVerificationRequest = verificationStage();
  check(
    "verification staging rejects exactly one missing selected seat request",
    missingOneVerificationRequest.status === 2 &&
      /verification request directory must contain exactly one readable JSON request per selected seat/.test(
        missingOneVerificationRequest.text,
      ) &&
      !existsSync(join(repo, ".scratch", "panel", "spec001w1-r2", ".complete")),
    missingOneVerificationRequest.text,
  );
  writeFileSync(removedVerificationRequest, removedVerificationBytes);

  const badFullSelection = readJson(currentSelectionPath);
  badFullSelection.classification_inputs.changed_paths = ["Makefile"];
  badFullSelection.classification_inputs.full_candidate.changed_paths = ["Makefile"];
  const badFullSelectionPath = join(repo, "bad-full-selection.json");
  writeJson(badFullSelectionPath, badFullSelection);
  const fullRangeMismatch = run(repo, [
    base,
    firstTip,
    "spec001w1-r2",
    "--selection",
    badFullSelectionPath,
    "--candidate",
    currentCandidatePath,
    "--ledger",
    continuationLedgerPath,
    "--responses",
    continuationCompletedResponsesPath,
    "--handoff",
    continuationHandoffPath,
    "--self-verification",
    stagedSelfVerification,
    "--verification-dir",
    continuationVerificationDir,
  ]);
  check(
    "verification staging compares base-to-tip full paths",
    fullRangeMismatch.status === 2 &&
      /full-candidate paths do not match git range/.test(fullRangeMismatch.text),
    fullRangeMismatch.text,
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
    continuationLedgerPath,
    "--responses",
    continuationCompletedResponsesPath,
    "--handoff",
    continuationHandoffPath,
    "--self-verification",
    stagedSelfVerification,
    "--verification-dir",
    continuationVerificationDir,
  ]);
  check("later review stages successfully", second.status === 0, second.text);
  const secondDir = join(repo, ".scratch", "panel", "spec001w1-r2");
  const secondRequest = readFileSync(join(secondDir, "review-request.md"), "utf8");
  check(
    "later request names the exact incremental range and verification artifacts",
    secondRequest.includes(`Delta range: \`${firstTip}..${secondTip}\``) &&
      secondRequest.includes("Immutable discovery ledger:") &&
      secondRequest.includes("Approval output after verdict collection:"),
  );
  check(
    "verification staging materializes and binds the finalized handoff",
    readFileSync(join(secondDir, "handoff.json"), "utf8") ===
      readFileSync(continuationHandoffPath, "utf8") &&
      readJson(join(secondDir, ".complete")).artifact_sha256["handoff.json"] ===
        digest(readFileSync(join(secondDir, "handoff.json"))),
  );
  check(
    "verification request documents the exact non-empty late-finding shape",
    secondRequest.includes("introduced_regression") &&
      secondRequest.includes("previously_missed") &&
      secondRequest.includes("source_ordinal"),
  );
  check(
    "verification staging preserves each request exactly",
    verificationRoster.every((seat) =>
      readFileSync(join(secondDir, "verification", `${seat}.json`), "utf8") ===
        readFileSync(join(continuationVerificationDir, `${seat}.json`), "utf8"),
    ),
  );

  const followOnWithoutHandoff = run(repo, [
    base,
    secondTip,
    "spec001w1-r3",
    "--selection",
    currentSelectionPath,
    "--candidate",
    currentCandidatePath,
    "--ledger",
    continuationLedgerPath,
    "--responses",
    continuationCompletedResponsesPath,
    "--self-verification",
    stagedSelfVerification,
    "--verification-dir",
    continuationVerificationDir,
  ]);
  check(
    "verification staging requires --handoff after a verification predecessor",
    followOnWithoutHandoff.status === 2 &&
      /requires --handoff when the previous selection phase is verification/.test(
        followOnWithoutHandoff.text,
      ) &&
      !existsSync(join(repo, ".scratch", "panel", "spec001w1-r3", ".complete")),
    followOnWithoutHandoff.text,
  );

  rmSync(join(secondDir, ".complete"));
  const incompleteRetry = run(repo, [
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
    "an incomplete packet blocks retry without automatic cleanup",
    incompleteRetry.status === 2 &&
      /already has an incomplete packet/.test(incompleteRetry.text) &&
      existsSync(secondDir),
    incompleteRetry.text,
  );
} finally {
  rmSync(repo, { recursive: true, force: true });
}

if (failures > 0) {
  console.error(`\ntest-stage-diffs: ${failures} failure(s)`);
  process.exit(1);
}
console.log("\ntest-stage-diffs: all cases passed");
