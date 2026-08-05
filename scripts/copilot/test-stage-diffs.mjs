#!/usr/bin/env node
// Coverage for the staged panel review request. The integrator dispatches the
// generated prompt verbatim, so this test proves that the prompt carries the
// incremental range, full context, evidence, prior-verdict obligation, and
// no-rerun rule instead of relying on a hand-written task prompt.

import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
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

let failures = 0;
const check = (name, ok, detail = "") => {
  if (ok) {
    console.log(`  ok   ${name}`);
    return;
  }
  failures += 1;
  console.error(`  FAIL ${name}${detail ? `: ${detail}` : ""}`);
};

function run(cwd, args) {
  const result = spawnSync("bash", [script, ...args], {
    cwd,
    encoding: "utf8",
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
  for (const seat of ["software", "test"]) {
    writeFileSync(join(agents, `panel-${seat}.agent.md`), `name: ${seat}\n`);
  }

  writeFileSync(join(repo, "base.txt"), "base\n");
  git(repo, "add", "base.txt", ".github/agents");
  git(repo, "commit", "--quiet", "-m", "base");
  const base = git(repo, "rev-parse", "HEAD");

  writeFileSync(join(repo, "first.txt"), "first change\n");
  git(repo, "add", "first.txt");
  git(repo, "commit", "--quiet", "-m", "first");
  const firstTip = git(repo, "rev-parse", "HEAD");

  console.log("stage-diffs: first review");
  const first = run(repo, [base, base, "spec001w1-r1"]);
  check("first review stages successfully", first.status === 0, first.text);

  const firstDir = join(repo, ".scratch", "panel", "spec001w1-r1");
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

  for (const seat of ["software", "test"]) {
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

  writeFileSync(join(repo, "second.txt"), "second change\n");
  git(repo, "add", "second.txt");
  git(repo, "commit", "--quiet", "-m", "second");
  const secondTip = git(repo, "rev-parse", "HEAD");

  console.log("stage-diffs: fail-closed continuity");
  const wrongPreviousTip = run(repo, [base, base, "spec001w1-r2"]);
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
  const incompletePreviousReview = run(repo, [base, firstTip, "spec001w1-r2"]);
  check(
    "later review rejects a missing prior seat verdict",
    incompletePreviousReview.status === 2 &&
      incompletePreviousReview.text.includes(
        "missing previous verdict for seat test",
      ),
    incompletePreviousReview.text,
  );
  writeFileSync(missingVerdict, savedVerdict);

  console.log("stage-diffs: incremental review");
  const second = run(repo, [base, firstTip, "spec001w1-r2"]);
  check("later review stages successfully", second.status === 0, second.text);

  const secondDir = join(repo, ".scratch", "panel", "spec001w1-r2");
  const delta = readFileSync(join(secondDir, "delta.diff"), "utf8");
  const full = readFileSync(join(secondDir, "full.diff"), "utf8");
  const secondRequest = readFileSync(join(secondDir, "review-request.md"), "utf8");
  check(
    "incremental diff excludes earlier changed paths",
    delta.includes("second.txt") && !delta.includes("first.txt"),
  );
  check(
    "full diff retains all branch changes",
    full.includes("first.txt") && full.includes("second.txt"),
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

  writeFileSync(
    join(secondDir, "verdicts", "software.json"),
    "{}\n",
  );
  const reused = run(repo, [base, firstTip, "spec001w1-r2"]);
  check(
    "a review id with verdicts cannot be restaged",
    reused.status === 2 &&
      reused.text.includes("already has verdicts"),
    reused.text,
  );
} finally {
  rmSync(repo, { recursive: true, force: true });
}

if (failures > 0) {
  console.error(`\ntest-stage-diffs: ${failures} failure(s)`);
  process.exit(1);
}
console.log("\ntest-stage-diffs: all cases passed");
