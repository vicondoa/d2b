#!/usr/bin/env node
// Validate that every Copilot agent in this repo has an explicit, legal, and
// self-consistent model / effort / context binding.
//
//   node scripts/copilot/check-bindings.mjs
//
// Why this exists. Copilot CLI 1.0.75 was measured to behave as follows:
//
//   * `model:` in agent frontmatter is honoured.
//   * `effortLevel:` and `contextTier:` in frontmatter are warned and ignored.
//   * `reasoningEffort:` in frontmatter is accepted with NO warning and is
//     completely inert. That is the dangerous shape: it looks applied.
//   * A subagent does NOT inherit the session's reasoning effort. An unpinned
//     lane runs at the model default, which is `medium`.
//   * Repo-scope `.github/copilot/settings.json` cannot carry `subagents`.
//
// So the only working per-lane binding is the dispatch parameters written in
// the skill tables, and a panel record attests `reasoning_effort`. A lane
// dispatched without an explicit effort therefore produces a false
// attestation rather than an error. This script is the cheap place to catch
// the mistakes that lead there.

import { existsSync, lstatSync, readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const agentsDir = join(root, ".github", "agents");
const skillsDir = join(root, ".github", "skills");
const modelRs = join(root, "packages", "xtask", "src", "delivery", "model.rs");

// Measured from the CLI's own model catalog. `gemini-3.1-pro-preview` has no
// `xhigh`; requesting it is invalid rather than merely unusual.
const CAPABILITIES = {
  "claude-opus-5": { efforts: ["low", "medium", "high", "xhigh", "max"], tiers: ["default", "long_context"] },
  "claude-opus-4.8": { efforts: ["low", "medium", "high", "xhigh", "max"], tiers: ["default", "long_context"] },
  "claude-sonnet-5": { efforts: ["low", "medium", "high", "xhigh", "max"], tiers: ["default", "long_context"] },
  "gpt-5.6-sol": { efforts: ["low", "medium", "high", "xhigh", "max"], tiers: ["default", "long_context"] },
  "gpt-5.6-terra": { efforts: ["low", "medium", "high", "xhigh", "max"], tiers: ["default", "long_context"] },
  "gpt-5.6-luna": { efforts: ["low", "medium", "high", "xhigh", "max"], tiers: ["default", "long_context"] },
  "gemini-3.1-pro-preview": { efforts: ["low", "medium", "high"], tiers: ["default", "long_context"] },
};

// Every spelling of effort or tier that is inert in frontmatter. Listing the
// silently-accepted spelling is the point: a warned-and-ignored key is
// self-announcing, an accepted-and-inert one is not.
const FORBIDDEN_FRONTMATTER = [
  "effortLevel", "effort_level", "effort",
  "reasoningEffort", "reasoning_effort",
  "contextTier", "context_tier",
];

const COPILOT_INTEGRATION = "copilot";
// Keep the retired integration token out of supported configuration while
// still detecting it if a stale file brings it back.
const RETIRED_INTEGRATION = ["open", "code"].join("");
const EXPECTED_COPILOT_SKILL_PATH = /^\.github\/skills\/[^/]+\/SKILL\.md$/;

const errors = [];
const fail = (m) => errors.push(m);

// Fails closed on every path. A gate that cannot read its own policy has not
// checked the binding; it has only declined to look. So an unreadable or
// reshaped `model.rs` is an error rather than a warning, and no downstream
// check is guarded by the policy having been read.
function readPolicy() {
  if (!existsSync(modelRs)) {
    fail(
      `cannot read policy constants: ${modelRs} not found. This gate attests that ` +
      `panel rows match the sealed policy; without the policy it enforces nothing. Restore \n      the file, or correct the path this gate reads.`,
    );
    return null;
  }
  const src = readFileSync(modelRs, "utf8");
  const pick = (name) => {
    const m = src.match(new RegExp(`${name}:\\s*&str\\s*=\\s*"([^"]+)"`));
    if (!m) {
      fail(
        `${modelRs}: cannot parse "${name}". The constant was renamed or reshaped, ` +
        `so this gate can no longer compare panel rows against it. Update the ` +
        `pattern in check-bindings.mjs in the same change.`,
      );
      return null;
    }
    return m[1];
  };
  const roles = [];
  const rolesBlock = src.match(/PANEL_ROLES[^=]*=\s*\[([\s\S]*?)\];/);
  if (rolesBlock) {
    for (const m of rolesBlock[1].matchAll(/PanelRole::(\w+)/g)) {
      roles.push(m[1].replace(/([a-z0-9])([A-Z])/g, "$1-$2").toLowerCase());
    }
  } else {
    fail(`${modelRs}: cannot parse PANEL_ROLES; the seat roster cannot be checked. Restore the PANEL_ROLES array, or update the pattern in check-bindings.mjs if it was reshaped deliberately.`);
  }
  return {
    provider: pick("PANEL_PROVIDER_POLICY"),
    model: pick("PANEL_MODEL_POLICY"),
    effort: pick("PANEL_REASONING_EFFORT_POLICY"),
    roles,
  };
}

function parseFrontmatter(text, label) {
  if (!text.startsWith("---\n")) {
    fail(`${label}: no YAML frontmatter. Begin the file with a "---" line, the name, description, model and tools keys, and a closing "---".`);
    return null;
  }
  const end = text.indexOf("\n---\n", 3);
  if (end === -1) {
    fail(`${label}: unterminated frontmatter. Add the closing "---" line above the prompt body.`);
    return null;
  }
  const out = {};
  for (const raw of text.slice(4, end).split("\n")) {
    if (!raw.trim() || raw.startsWith("#") || /^\s/.test(raw)) continue;
    const i = raw.indexOf(":");
    if (i === -1) continue;
    out[raw.slice(0, i).trim()] = raw.slice(i + 1).trim();
  }
  return out;
}

// --- agents ---------------------------------------------------------------

const agents = new Map();
if (!existsSync(agentsDir)) {
  fail(`${agentsDir} does not exist. Copilot discovers agents only there, so every role is unbound. Restore the directory.`);
} else {
  for (const file of readdirSync(agentsDir).sort()) {
    if (!file.endsWith(".agent.md")) continue;
    const name = file.slice(0, -".agent.md".length);
    const text = readFileSync(join(agentsDir, file), "utf8");
    const fm = parseFrontmatter(text, file);
    if (!fm) continue;

    if (fm.name !== name) {
      fail(`${file}: frontmatter name "${fm.name}" does not match the file basename "${name}". Change one to match the other; dispatch resolves the agent by basename.`);
    }
    if (!fm.description) fail(`${file}: description is required for dispatch selection. Add a "description:" line saying what this agent reviews or does.`);

    for (const key of FORBIDDEN_FRONTMATTER) {
      if (key in fm) {
        fail(
          `${file}: frontmatter carries "${key}". No spelling of effort or context tier ` +
          `works in agent frontmatter on Copilot CLI 1.0.75. "reasoningEffort" in ` +
          `particular is accepted without a warning and does nothing, which reads as ` +
          `authoritative and is not. Put the value in the skill's dispatch table instead.`,
        );
      }
    }
    if (!fm.model) {
      fail(
        `${file}: no "model:" in frontmatter. An agent without one, invoked without ` +
        `dispatch parameters, inherits the PARENT session's model, so a panel seat ` +
        `would run on the architect's model and be attested as Gemini. Add a ` +
        `"model:" line naming the model this agent must run on.`,
      );
    } else if (!CAPABILITIES[fm.model]) {
      fail(
        `${file}: model "${fm.model}" is not in the capability table, so its effort and ` +
        `context tier cannot be checked. Either the model name is wrong, or the table ` +
        `needs the new model added with its real effort and tier ceilings. Skipping the ` +
        `check silently is how an illegal effort reaches a dispatch and downgrades.`,
      );
    }
    if (name.startsWith("panel-")) {
      const tools = fm.tools ?? "";
      if (/\b(bash|edit|create|write|task|sql)\b/.test(tools)) {
        fail(
          `${file}: panel agents are read-only by construction. "tools:" must not grant ` +
          `${tools}. Reviewers read staged diffs; granting shell also puts ten lanes on ` +
          `the shared Nix store and the heavy-gate semaphore. Remove those entries from \n          "tools:".`,
        );
      }
      if (!/\bview\b/.test(tools)) {
        fail(`${file}: panel agent needs "view" to read the staged diffs. Add view to its "tools:" list.`);
      }
    }
    agents.set(name, { file, model: fm.model, tools: fm.tools ?? "", text });
  }
}

// --- the shared finding bar -----------------------------------------------
// Every panel seat must carry one byte-identical statement of what qualifies
// as a blocking finding. This is checked mechanically because prose alone did
// not hold: the bar was originally written once and restated per seat, and it
// silently diverged into ten different thresholds. Three seats ended up with
// no threshold at all, so anything they noticed became a blocking
// recommendation, and since signoff is true iff recommendations is empty,
// each one cost a full extra round across all ten seats.

const BAR_HEADING = "## The bar for a finding";
const BAR_NEXT_HEADING = "## Output";

// The extent is pinned to exactly one following heading rather than "whatever
// H2 comes next", and the start must be a unique heading on its own line.
// A looser boundary fails OPEN, which is the one failure this gate cannot
// have: a seat could inject its own section between the bar and `## Output`,
// or embed an H2 inside the bar, and the compared slice would still match
// every other seat byte for byte while that seat actually read a different
// threshold. Matching a bare substring anywhere in the file has the same
// shape, since a mention inside a fenced block would anchor the slice to the
// wrong place and leave the real bar unchecked.
const headingOffsets = (text, heading) => {
  const literal = heading.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const re = new RegExp(`(^|\\r?\\n)(${literal})[ \\t]*(?=\\r?\\n|$)`, "g");
  const offsets = [];
  for (let m = re.exec(text); m !== null; m = re.exec(text)) {
    offsets.push(m.index + m[1].length);
  }
  return offsets;
};

const panelAgents = [...agents.entries()].filter(([n]) => n.startsWith("panel-"));

const bars = new Map();
for (const [name, a] of panelAgents) {
  const starts = headingOffsets(a.text, BAR_HEADING);
  if (starts.length === 0) {
    fail(
      `${a.file}: no "${BAR_HEADING}" section. Every panel seat must carry the shared ` +
      `bar verbatim, or that seat invents its own threshold and reports below it. ` +
      `Copy the section from another panel agent without changing a word.`,
    );
    continue;
  }
  if (starts.length > 1) {
    fail(
      `${a.file}: ${starts.length} "${BAR_HEADING}" sections. Exactly one is allowed, ` +
      `because only the first would be compared and a later one could state a ` +
      `different threshold unchecked. Delete the duplicates.`,
    );
    continue;
  }
  const start = starts[0];
  const end = headingOffsets(a.text, BAR_NEXT_HEADING).find((i) => i > start);
  if (end === undefined) {
    fail(
      `${a.file}: "${BAR_HEADING}" is not followed by "${BAR_NEXT_HEADING}", so its ` +
      `extent is undefined. The bar is compared up to that heading; without it there ` +
      `is no agreed end and trailing text would go unchecked. Keep "${BAR_NEXT_HEADING}" after it.`,
    );
    continue;
  }
  const section = a.text.slice(start, end);
  const inner = [...section.matchAll(/(?:^|\r?\n)(## [^\r\n]*)/g)]
    .map((m) => m[1].trim())
    .slice(1);
  if (inner.length > 0) {
    fail(
      `${a.file}: unexpected section(s) between "${BAR_HEADING}" and ` +
      `"${BAR_NEXT_HEADING}": ${inner.join(", ")}. Nothing may sit between them. An ` +
      `injected heading ends the compared slice early, so the seat matches every ` +
      `other seat while reading extra instructions the gate never saw.`,
    );
    continue;
  }
  bars.set(name, section);
}

if (bars.size > 1) {
  const [refName, refBar] = [...bars.entries()][0];
  for (const [name, bar] of bars) {
    if (bar !== refBar) {
      fail(
        `${agents.get(name).file}: its "${BAR_HEADING}" section differs from ` +
        `${agents.get(refName).file}. All ten seats must apply one identical bar; a ` +
        `per-seat variant is how the panel starts returning findings at ten different ` +
        `severities. Make them byte-identical.`,
      );
    }
  }
}

// --- skill binding tables -------------------------------------------------

const rows = [];
if (existsSync(skillsDir)) {
  for (const skill of readdirSync(skillsDir).sort()) {
    const path = join(skillsDir, skill, "SKILL.md");
    if (!existsSync(path)) continue;
    for (const line of readFileSync(path, "utf8").split("\n")) {
      if (!line.trim().startsWith("|")) continue;
      const cells = line.split("|").slice(1, -1).map((c) => c.trim().replace(/^`|`$/g, ""));
      if (cells.length < 5) continue;
      const [, agent, model, effort, tier] = cells;
      if (!agents.has(agent)) continue;
      rows.push({ skill, agent, model, effort, tier, line: line.trim() });
    }
  }
}

const bound = new Set(rows.map((r) => r.agent));
for (const name of agents.keys()) {
  if (!bound.has(name)) {
    fail(
      `agent "${name}" has no binding row in any .github/skills/*/SKILL.md table. ` +
      `Every agent must be dispatched with an explicit model, reasoning_effort and ` +
      `context_tier; an unbound agent will silently run at the model default effort. ` +
      `Add a row for this agent to the dispatch table in the skill that dispatches it.`,
    );
  }
}

const policy = readPolicy();

for (const r of rows) {
  const a = agents.get(r.agent);
  if (a.model && r.model !== a.model) {
    fail(
      `${r.skill}/SKILL.md: row for "${r.agent}" pins model "${r.model}" but ` +
      `${a.file} frontmatter pins "${a.model}". These must agree. Change whichever ` +
      `is wrong; the row is what the dispatch actually uses.`,
    );
  }
  const caps = CAPABILITIES[r.model];
  if (!caps) {
    fail(
      `${r.skill}/SKILL.md: model "${r.model}" for "${r.agent}" is not in the capability ` +
      `table, so its effort and context tier cannot be checked. Add the model with its ` +
      `real ceilings rather than leaving the row unchecked.`,
    );
    continue;
  }
  if (!caps.efforts.includes(r.effort)) {
    fail(
      `${r.skill}/SKILL.md: reasoning_effort "${r.effort}" is not valid for "${r.model}" ` +
      `(valid: ${caps.efforts.join(", ")}). The observed failure mode for an invalid ` +
      `effort is a silent downgrade, not an error. Change the row to one of the ` +
      `valid levels.`,
    );
  }
  if (!caps.tiers.includes(r.tier)) {
    fail(
      `${r.skill}/SKILL.md: context_tier "${r.tier}" is not valid for "${r.model}" ` +
      `(valid: ${caps.tiers.join(", ")}). Change the row to one of those tiers.`,
    );
  }
  if (policy && r.agent.startsWith("panel-")) {
    if (policy.model && r.model !== policy.model) {
      fail(
        `${r.skill}/SKILL.md: panel row "${r.agent}" pins model "${r.model}" but ` +
        `PANEL_MODEL_POLICY is "${policy.model}". panel-attest would reject those ` +
        `records. Change the row to the policy model.`,
      );
    }
    if (policy.effort && r.effort !== policy.effort) {
      fail(
        `${r.skill}/SKILL.md: panel row "${r.agent}" pins effort "${r.effort}" but ` +
        `PANEL_REASONING_EFFORT_POLICY is "${policy.effort}". Change the row to the ` +
        `policy effort.`,
      );
    }
  }
}

// Every roster seat must have an agent.
if (policy && policy.roles.length) {
  for (const role of policy.roles) {
    if (!agents.has(`panel-${role}`)) {
      fail(`PANEL_ROLES names seat "${role}" but there is no .github/agents/panel-${role}.agent.md. Add that agent, or remove the seat from PANEL_ROLES.`);
    }
  }
  for (const name of agents.keys()) {
    if (name.startsWith("panel-") && !policy.roles.includes(name.slice("panel-".length))) {
      fail(`agent "${name}" is not a seat in PANEL_ROLES; the roster is closed. Remove the agent, or add the seat to PANEL_ROLES in the same change.`);
    }
  }
}

// A committed repo-scope settings file cannot carry these keys.
const repoSettings = join(root, ".github", "copilot", "settings.json");
if (existsSync(repoSettings)) {
  const text = readFileSync(repoSettings, "utf8");
  for (const key of ["subagents", "effortLevel", "contextTier"]) {
    if (text.includes(`"${key}"`)) {
      fail(
        `.github/copilot/settings.json carries "${key}", which repo-scope settings do not ` +
        `honour. The CLI filters repo scope through a fixed allowlist that excludes it, so ` +
        `this file would silently govern nothing. Remove the key, and pin the binding at \n        dispatch in the skill table instead.`,
      );
    }
  }
}

function retiredPathExists(path) {
  try {
    lstatSync(path);
    return true;
  } catch (e) {
    if (e.code === "ENOENT") return false;
    fail(`cannot inspect ${path}: ${e.message}. The Copilot-only surface check cannot continue safely.`);
    return false;
  }
}

const retiredSurfacePaths = [
  {
    label: "retired integration directory",
    path: join(root, `.${RETIRED_INTEGRATION}`),
  },
  {
    label: "retired integration manifest",
    path: join(root, ".specify", "integrations", `${RETIRED_INTEGRATION}.manifest.json`),
  },
];
for (const { label, path } of retiredSurfacePaths) {
  if (retiredPathExists(path)) {
    fail(`${label} is present at ${path}. Remove the retired surface; Copilot is the only supported integration.`);
  }
}

const copilotManifestJson = join(root, ".specify", "integrations", "copilot.manifest.json");
if (!existsSync(copilotManifestJson)) {
  fail(
    `.specify/integrations/copilot.manifest.json does not exist. The installed ` +
    `Copilot skill surface cannot be checked without its manifest.`,
  );
} else {
  let manifest = null;
  let parsed = false;
  try {
    manifest = JSON.parse(readFileSync(copilotManifestJson, "utf8"));
    parsed = true;
  } catch (e) {
    fail(
      `.specify/integrations/copilot.manifest.json is not valid JSON: ${e.message}. ` +
      `Repair the manifest before checking the installed Copilot skill surface.`,
    );
  }
  if (parsed) {
    if (typeof manifest !== "object" || Array.isArray(manifest)) {
      fail(
        `.specify/integrations/copilot.manifest.json must contain a JSON object ` +
        `describing the installed Copilot skill surface.`,
      );
    } else {
      if (manifest.integration !== COPILOT_INTEGRATION) {
        fail(
          `.specify/integrations/copilot.manifest.json integration must be ` +
          `"${COPILOT_INTEGRATION}"; found ${JSON.stringify(manifest.integration)}.`,
        );
      }
      const files = manifest.files;
      if (!files || typeof files !== "object" || Array.isArray(files)) {
        fail(
          `.specify/integrations/copilot.manifest.json files must be an object ` +
          `of required Copilot skill paths.`,
        );
      } else {
        const declared = Object.keys(files);
        if (declared.length === 0) {
          fail(
            `.specify/integrations/copilot.manifest.json declares no required ` +
            `Copilot skill files.`,
          );
        }
        for (const relativePath of declared) {
          if (!EXPECTED_COPILOT_SKILL_PATH.test(relativePath)) {
            fail(
              `.specify/integrations/copilot.manifest.json declares "${relativePath}", ` +
              `which is not an expected Copilot skill path; use ` +
              `.github/skills/<name>/SKILL.md.`,
            );
            continue;
          }
          const skillPath = join(root, relativePath);
          let skillStat;
          try {
            skillStat = statSync(skillPath);
          } catch (e) {
            fail(
              `.specify/integrations/copilot.manifest.json declares required Copilot ` +
              `skill "${relativePath}", but the path does not resolve to a ` +
              `readable regular file: ${e.message}.`,
            );
            continue;
          }
          if (!skillStat.isFile()) {
            fail(
              `.specify/integrations/copilot.manifest.json declares required Copilot ` +
              `skill "${relativePath}", but the path is not a regular file.`,
            );
            continue;
          }
          try {
            readFileSync(skillPath, "utf8");
          } catch (e) {
            fail(
              `.specify/integrations/copilot.manifest.json declares required Copilot ` +
              `skill "${relativePath}", but the file is not readable: ${e.message}.`,
            );
          }
        }
      }
    }
  }
}

// spec-kit integration state. `specify init` replaces installed_integrations
// rather than appending, so a later initialization can silently select a
// different integration. This repository is Copilot-only: the installed,
// current, default, and configured integration must all resolve to Copilot.
function findRetiredPaths(value, path = "$", paths = []) {
  if (Array.isArray(value)) {
    value.forEach((item, index) => findRetiredPaths(item, `${path}[${index}]`, paths));
    return paths;
  }
  if (value && typeof value === "object") {
    for (const [key, item] of Object.entries(value)) {
      const keyPath = `${path}.${key}`;
      if (key.toLowerCase() === RETIRED_INTEGRATION) {
        paths.push(`${keyPath} (key)`);
      }
      findRetiredPaths(item, keyPath, paths);
    }
    return paths;
  }
  if (typeof value === "string" && value.trim().toLowerCase() === RETIRED_INTEGRATION) {
    paths.push(path);
  }
  return paths;
}

const integrationJson = join(root, ".specify", "integration.json");
if (!existsSync(integrationJson)) {
  fail(
    `.specify/integration.json does not exist. Copilot is the only supported ` +
    `integration, so the state file is required and cannot be skipped.`,
  );
} else {
  let state = null;
  let parsed = false;
  try {
    state = JSON.parse(readFileSync(integrationJson, "utf8"));
    parsed = true;
  } catch (e) {
    fail(`.specify/integration.json is not valid JSON: ${e.message}. Repair the file; the Copilot-only integration check cannot run without it.`);
  }
  if (parsed) {
    if (typeof state !== "object" || Array.isArray(state)) {
      fail(
        `.specify/integration.json must contain a JSON object describing the ` +
        `Copilot integration state.`,
      );
    } else {
      const retiredPaths = findRetiredPaths(state);
      if (retiredPaths.length) {
        fail(
          `.specify/integration.json contains the retired integration at ` +
          `${retiredPaths.join(", ")}. Remove it; Copilot is the sole supported ` +
          `integration and stale integration state must fail closed.`,
        );
      }
      const installed = state.installed_integrations;
      if (
        !Array.isArray(installed) ||
        installed.length !== 1 ||
        installed[0] !== COPILOT_INTEGRATION
      ) {
        fail(
          `.specify/integration.json installed_integrations must be exactly ` +
          `["${COPILOT_INTEGRATION}"]; found ${JSON.stringify(installed)}. ` +
          `The repository has one supported integration.`,
        );
      }
      const settings = state.integration_settings;
      const settingKeys =
        settings && typeof settings === "object" && !Array.isArray(settings)
          ? Object.keys(settings)
          : null;
      if (
        !settingKeys ||
        settingKeys.length !== 1 ||
        settingKeys[0] !== COPILOT_INTEGRATION
      ) {
        fail(
          `.specify/integration.json integration_settings must contain only ` +
          `"${COPILOT_INTEGRATION}"; found ${JSON.stringify(settingKeys)}. ` +
          `Remove stale integration settings.`,
        );
      }
      for (const key of ["integration", "default_integration"]) {
        if (state[key] !== COPILOT_INTEGRATION) {
          fail(
            `.specify/integration.json ${key} must be "${COPILOT_INTEGRATION}"; ` +
            `found ${JSON.stringify(state[key])}. The repository must resolve ` +
            `current and default operations to Copilot.`,
          );
        }
      }
    }
  }
}

const initOptionsJson = join(root, ".specify", "init-options.json");
if (!existsSync(initOptionsJson)) {
  fail(
    `.specify/init-options.json does not exist. Copilot must remain the only ` +
    `initialization and current integration.`,
  );
} else {
  let options = null;
  let parsed = false;
  try {
    options = JSON.parse(readFileSync(initOptionsJson, "utf8"));
    parsed = true;
  } catch (e) {
    fail(`.specify/init-options.json is not valid JSON: ${e.message}. Repair the file before initialization can be checked.`);
  }
  if (parsed) {
    if (typeof options !== "object" || Array.isArray(options)) {
      fail(`.specify/init-options.json must contain a JSON object selecting Copilot.`);
    } else {
      const retiredPaths = findRetiredPaths(options);
      if (retiredPaths.length) {
        fail(
          `.specify/init-options.json contains the retired integration at ` +
          `${retiredPaths.join(", ")}. Remove it; initialization must select Copilot.`,
        );
      }
      for (const key of ["ai", "integration"]) {
        if (options[key] !== COPILOT_INTEGRATION) {
          fail(
            `.specify/init-options.json ${key} must be "${COPILOT_INTEGRATION}"; ` +
            `found ${JSON.stringify(options[key])}.`,
          );
        }
      }
    }
  }
}

// --- record-helper constant drift -----------------------------------------
//
// make-records.mjs mirrors constants that are canonically defined in Rust. It
// runs only during a live panel round, so a drifted copy would not surface
// until the moment a wave is being sealed - the worst possible time and the
// one place a wrong value becomes a false attestation. Pin them here, where
// they are checked on every lint run.
{
  const helper = join(root, ".github", "skills", "d2b-panel-round", "scripts", "make-records.mjs");
  const panelRs = join(root, "packages", "xtask", "src", "delivery", "panel.rs");
  const modRs = join(root, "packages", "xtask", "src", "delivery", "mod.rs");

  if (!existsSync(helper)) {
    fail(`cannot read ${helper}; the panel record helper is required. Restore it; the drift pin cannot be checked without it.`);
  } else {
    const src = readFileSync(helper, "utf8");
    const num = (name) => {
      const m = src.match(new RegExp(`const\\s+${name}\\s*=\\s*(\\d+)`));
      if (!m) {
        fail(`make-records.mjs: cannot parse ${name}; the drift check cannot verify it. Restore the constant, or update the pattern in check-bindings.mjs in the same change.`);
        return null;
      }
      return Number(m[1]);
    };
    const str = (name) => {
      const m = src.match(new RegExp(`const\\s+${name}\\s*=\\s*"([^"]+)"`));
      if (!m) {
        fail(`make-records.mjs: cannot parse ${name}; the drift check cannot verify it. Restore the constant, or update the pattern in check-bindings.mjs in the same change.`);
        return null;
      }
      return m[1];
    };
    // Read each canonical value from the Rust file that actually defines it.
    const rustStr = (file, label, name) => {
      if (!existsSync(file)) { fail(`cannot read ${label}; drift check cannot run. Restore the file.`); return null; }
      const m = readFileSync(file, "utf8").match(new RegExp(`${name}:\\s*&str\\s*=\\s*"([^"]+)"`));
      if (!m) { fail(`${label}: cannot parse ${name}; drift check cannot run. Restore the constant, or update the pattern in check-bindings.mjs in the same change.`); return null; }
      return m[1];
    };
    const rustNum = (file, label, name) => {
      if (!existsSync(file)) { fail(`cannot read ${label}; drift check cannot run. Restore the file.`); return null; }
      const m = readFileSync(file, "utf8").match(new RegExp(`${name}:\\s*(?:usize|u32)\\s*=\\s*([0-9*\\s]+);`));
      if (!m) { fail(`${label}: cannot parse ${name}; drift check cannot run. Restore the constant, or update the pattern in check-bindings.mjs in the same change.`); return null; }
      // Tolerate the `4 * 1024` spelling without evaluating arbitrary source.
      const parts = m[1].split("*").map((p) => Number(p.trim()));
      if (parts.some((p) => !Number.isFinite(p))) {
        fail(`${label}: ${name} is not a simple integer product; drift check cannot run. Spell it as an integer or a product of integers, or teach check-bindings.mjs the new form.`);
        return null;
      }
      return parts.reduce((a, b) => a * b, 1);
    };

    const mirrors = [
      ["ARTIFACT_KIND", str("ARTIFACT_KIND"), rustStr(modelRs, "model.rs", "PANEL_ATTESTATION_ARTIFACT_KIND")],
      ["SCHEMA_VERSION", num("SCHEMA_VERSION"), rustNum(modRs, "mod.rs", "DELIVERY_SCHEMA_VERSION")],
      ["MAX_RECOMMENDATIONS", num("MAX_RECOMMENDATIONS"), rustNum(panelRs, "panel.rs", "MAX_RECOMMENDATIONS")],
    ];
    for (const [name, mine, canonical] of mirrors) {
      if (mine === null || canonical === null) continue;
      if (String(mine) !== String(canonical)) {
        fail(
          `make-records.mjs ${name} is ${JSON.stringify(mine)} but the canonical Rust ` +
          `value is ${JSON.stringify(canonical)}. A drifted copy is only discovered ` +
          `while sealing a wave, which is exactly when a wrong value becomes a false ` +
          `attestation. Update make-records.mjs to the canonical value.`,
        );
      }
    }

    // The panel policy strings the helper enforces must equal the sealed policy.
    if (policy) {
      const policyMirrors = [
        ["PROVIDER_POLICY", str("PROVIDER_POLICY"), policy.provider],
        ["MODEL_POLICY", str("MODEL_POLICY"), policy.model],
        ["EFFORT_POLICY", str("EFFORT_POLICY"), policy.effort],
      ];
      for (const [name, mine, canonical] of policyMirrors) {
        if (mine === null || canonical == null) continue;
        if (mine !== canonical) {
          fail(
            `make-records.mjs ${name} is "${mine}" but model.rs pins "${canonical}". ` +
            `The helper would attest a binding the gate does not accept. Update the helper constant.`,
          );
        }
      }

      // The seat roster is mirrored as an array rather than a scalar, so it
      // needs its own comparison. A helper roster short of the sealed one
      // writes an incomplete record set and the gate rejects the wave for a
      // missing seat; a longer one writes a record for a seat that is not on
      // the roster. Compare in order, because the two are in order today and
      // a reordering is itself drift worth surfacing.
      const rolesBlock = src.match(/const\s+ROLES\s*=\s*\[([\s\S]*?)\];/);
      if (!rolesBlock) {
        fail(
          `make-records.mjs: cannot parse ROLES; the seat-roster drift check cannot run. Restore the ROLES array, or update the pattern in check-bindings.mjs in the same change.`,
        );
      } else {
        const mineRoles = [...rolesBlock[1].matchAll(/"([^"]+)"/g)].map((m) => m[1]);
        if (mineRoles.join(",") !== policy.roles.join(",")) {
          fail(
            `make-records.mjs ROLES is [${mineRoles.join(", ")}] but model.rs ` +
            `PANEL_ROLES is [${policy.roles.join(", ")}]. A drifted roster is only ` +
            `discovered while sealing a wave, and it either drops a seat from the ` +
            `record set or attests one the gate does not accept. Bring the helper roster back into the sealed order.`,
          );
        }
      }
    }

    // The string ceilings need only be no looser than the Rust bound; a
    // stricter local cap is a deliberate choice, a looser one is a defect.
    const rustMaxBytes = rustNum(modelRs, "model.rs", "MAX_STRING_BYTES");
    for (const name of ["MAX_SUMMARY_CHARS", "MAX_RECOMMENDATION_CHARS"]) {
      const mine = num(name);
      if (mine === null || rustMaxBytes === null) continue;
      if (mine > rustMaxBytes) {
        fail(
          `make-records.mjs ${name} is ${mine}, looser than model.rs MAX_STRING_BYTES ` +
          `(${rustMaxBytes}). The helper would accept a value the sealing path rejects. Lower the helper cap to that bound or below.`,
        );
      }
    }
  }
}

// --- delivery memory taxonomy ---------------------------------------------
// The registers are a queryable classification surface, not prose. A
// free-form category silently degrades that: "filed-guard" and "filed" do
// not group, so the escalation rule ("a category recurring across three
// waves becomes a task") stops counting correctly and the register becomes
// the graveyard it exists to avoid. The vocabularies are closed in
// .github/skills/d2b-memory/SKILL.md and pinned here.
const MEMORY_CATEGORIES = ["signoff", "build", "test", "merge", "codegen", "disk"];
const MEMORY_DISPOSITIONS = ["open", "folded", "filed", "resolved", "wontfix"];

// A register with no rows is normally a register that lost them, so it fails.
// A genuinely empty one is declared with this marker, which makes emptiness a
// statement an author made rather than a state the gate cannot tell apart from
// truncation. The marker is refused once the register has rows again, so it
// cannot be left behind to license a later truncation.
const EMPTY_MARKER = "<!-- d2b-register: intentionally empty -->";

// The qualified wave grammar, as documented in docs/contributing/workflow.md
// and enforced by validate_wave in packages/xtask/src/delivery/. The program
// component carries no hyphen, which is the constraint most easily missed
// when a branch name is reused as a wave token.
const TARGET_WAVE = /^(W[0-8]|[a-z][a-z0-9]{2,15}w[0-8])$/;
// A row records where the observation happened, so a follow-up round is a
// legal origin. A fold target is a wave rather than a round, so the
// disposition column keeps the stricter pattern above.
const ORIGIN_WAVE = /^(W[0-8]|[a-z][a-z0-9]{2,15}w[0-8])(fu[1-9][0-9]?)?$/;

// Split one Markdown table row into cells.
//
// Cells may contain escapes, so the row is walked rather than pattern-matched:
// `\|` is a literal pipe, `\\` is a literal backslash and does not protect the
// pipe after it, and any other pipe is a separator. A statement quoting a
// shell pipeline relies on the first of those.
//
// Only an EMPTY leading or trailing cell is dropped, because those are the
// artefacts of the outer pipes. Markdown does not require a trailing pipe, so
// a non-empty last cell is real data and is kept.
function splitRow(line) {
  const cells = [];
  let cur = "";
  for (let i = 0; i < line.length; i += 1) {
    const ch = line[i];
    if (ch === "\\" && (line[i + 1] === "|" || line[i + 1] === "\\")) {
      cur += line[i + 1];
      i += 1;
      continue;
    }
    if (ch === "|") {
      cells.push(cur.trim());
      cur = "";
      continue;
    }
    cur += ch;
  }
  cells.push(cur.trim());
  if (cells.length && cells[0] === "") cells.shift();
  if (cells.length && cells[cells.length - 1] === "") cells.pop();
  return cells;
}

// A row is recognised by its leading pipe, so a row that lost one reads as
// prose and every column on it goes unvalidated. Two shapes identify such a
// row without claiming an ordinary pipe in prose is one:
//
//   - it ends with an unescaped pipe and carries more than one, which is how
//     every row in these registers is written; or
//   - it carries an unescaped pipe while a table is open above it, since a
//     line between two rows of a table is not prose.
//
// A pipe in a sentence or a shell pipeline matches neither, which matters
// because escaping one to satisfy this gate would corrupt the text it appears
// in. A table written with neither outer pipe, following a valid table, is not
// distinguishable from prose here and is out of scope; every register in this
// repo writes both.
function pipeShape(line) {
  const text = line.trim();
  let pipes = 0;
  let endsWithPipe = false;
  for (let i = 0; i < text.length; i += 1) {
    if (text[i] === "\\") { i += 1; continue; }
    if (text[i] !== "|") continue;
    pipes += 1;
    endsWithPipe = i === text.length - 1;
  }
  return { pipes, endsWithPipe };
}

const memoryDir = join(root, ".specify", "memory");
let registerRows = 0;
for (const reg of ["friction-log.md", "deferred-work.md", "engineering-debt.md"]) {
  const path = join(memoryDir, reg);
  if (!existsSync(path)) {
    fail(
      `.specify/memory/${reg}: this register is missing. All three are the memory ` +
      `this process runs on, so an absent one is an unrecorded gap rather than an ` +
      `empty register. Restore it, or create it with its header row.`,
    );
    continue;
  }
  if (!statSync(path).isFile()) {
    fail(
      `.specify/memory/${reg}: this path exists but is not a regular file, so its ` +
      `rows cannot be read. Replace it with the register file.`,
    );
    continue;
  }
  // Lone CR is accepted as a line ending rather than silently swallowed. A file
  // written that way collapses to a single line, so no row in it is read as a
  // row. Today that is caught incidentally: these registers open with a
  // markdown title, so the fused line does not start with a pipe and is either
  // refused as a lost row or leaves sawHeader false. Neither check is aimed at
  // this, and a register written without a leading title would fuse into a line
  // whose first cell is a valid header and pass with nothing behind it. Read the
  // lines correctly instead of relying on an incidental rejection.
  const lines = readFileSync(path, "utf8").split(/\r\n|\r|\n/);
  // The registers do not share a shape, so the disposition column is located by
  // its header rather than by position: deferred-work.md carries a trailing Ref
  // column that the others do not.
  //
  // Both the column index and the header's width describe one table, so both
  // reset at the first line that is not a row. Every row is then addressed
  // against the header above it or refused: a row before any header, a row of a
  // different width, and a file with no header at all are all rejected. There
  // is no fallback to a positional read, because a row read against the wrong
  // column is validated in appearance only.
  let dispositionIdx = -1;
  let headerWidth = -1;
  let sawHeader = false;
  let inFence = false;
  let rowsHere = 0;
  let fenceOpenedAt = -1;
  let declaredEmpty = false;
  for (const [i, line] of lines.entries()) {
    // A table cannot span a fence, and a fenced example may legitimately hold
    // pipes, so a fence closes the table above it and its contents are skipped.
    // Opening one mid-table is not that: it silently swallows the rows beneath
    // it, so it is refused rather than skipped.
    if (/^\s*(```|~~~)/.test(line)) {
      if (!inFence && dispositionIdx >= 0) {
        fail(
          `.specify/memory/${reg}:${i + 1}: a fenced block opens inside a table, so ` +
          `every row it encloses is skipped unread. Close the table with a blank ` +
          `line before the fence, or move the fence out of the table.`,
        );
      }
      inFence = !inFence;
      fenceOpenedAt = inFence ? i + 1 : -1;
      dispositionIdx = -1;
      headerWidth = -1;
      continue;
    }
    if (inFence) {
      dispositionIdx = -1;
      headerWidth = -1;
      continue;
    }
    if (line.trim() === EMPTY_MARKER) {
      declaredEmpty = true;
    }
    if (!line.trim().startsWith("|")) {
      const { pipes, endsWithPipe } = pipeShape(line);
      if ((pipes > 1 && endsWithPipe) || (pipes > 0 && dispositionIdx >= 0)) {
        fail(
          `.specify/memory/${reg}:${i + 1}: this line reads as a row that lost its ` +
          `leading pipe, so it is skipped as prose and no column on it is ` +
          `validated. Add the leading pipe. If the line is genuinely prose, separate ` +
          `it from the table with a blank line.`,
        );
      }
      dispositionIdx = -1;
      headerWidth = -1;
      continue;
    }
    const cells = splitRow(line);
    if (cells.length === 0) continue;
    if (cells.every((c) => /^:?-+:?$/.test(c))) continue;
    if (dispositionIdx < 0 && cells[0].toLowerCase() === "wave") {
      dispositionIdx = cells.findIndex((c) => c.toLowerCase() === "disposition");
      if (dispositionIdx < 0) {
        fail(
          `.specify/memory/${reg}:${i + 1}: the header row names no Disposition ` +
          `column, so no row in this register can be validated. Add a Disposition ` +
          `column to the header and a cell for it to every row beneath.`,
        );
      }
      headerWidth = cells.length;
      sawHeader = true;
      continue;
    }
    if (dispositionIdx < 0) {
      fail(
        `.specify/memory/${reg}:${i + 1}: this row precedes any header row, so ` +
        `there is no Disposition column to validate it against. Move it below ` +
        `the header, or add a header row above it.`,
      );
      continue;
    }
    if (cells.length !== headerWidth) {
      fail(
        `.specify/memory/${reg}:${i + 1}: this row has ${cells.length} cells and its ` +
        `header has ${headerWidth}, so its columns do not line up with the ones ` +
        `this register is validated against. Check for a column left out, a ` +
        `missing outer pipe, or a pipe inside a cell that needs escaping as \\|.`,
      );
      continue;
    }
    registerRows += 1;
    rowsHere += 1;
    const wave = cells[0].replace(/`/g, "");
    if (!wave) {
      fail(
        `.specify/memory/${reg}:${i + 1}: this row names no wave. Use the legacy ` +
        `closed set W0..W8, or a qualified token whose program component carries ` +
        `no hyphen (copilotw6, spec001w1, adr046w3fu2).`,
      );
    } else if (!ORIGIN_WAVE.test(wave)) {
      fail(
        `.specify/memory/${reg}:${i + 1}: wave "${wave}" is not a legal wave token. ` +
        `Use the legacy closed set W0..W8, or a qualified token whose program ` +
        `component carries no hyphen (copilotw6, spec001w1, adr046w3fu2).`,
      );
    }
    const category = cells[1].replace(/`/g, "");
    if (!category) {
      fail(
        `.specify/memory/${reg}:${i + 1}: this row names no category, so it groups ` +
        `with nothing and the three-wave escalation rule cannot count it. Use one of ` +
        `${MEMORY_CATEGORIES.join(", ")}.`,
      );
    } else if (!MEMORY_CATEGORIES.includes(category)) {
      fail(
        `.specify/memory/${reg}:${i + 1}: category "${category}" is not in the closed ` +
        `taxonomy (${MEMORY_CATEGORIES.join(", ")}). A near-miss spelling does not group ` +
        `with its siblings, so the three-wave escalation rule stops counting it. Change ` +
        `it to one of those categories.`,
      );
    }
    const disposition = cells[dispositionIdx].replace(/`/g, "");
    // A folded row records its target wave in the disposition column instead
    // of a vocabulary term. Both wave spellings are legal there: the legacy
    // closed set W0..W8, and the qualified token this repo now prefers. The
    // pattern is the grammar itself rather than a loose wildcard, so a
    // malformed wave is still caught.
    const known = MEMORY_DISPOSITIONS.includes(disposition);
    if (!disposition) {
      fail(
        `.specify/memory/${reg}:${i + 1}: this row names no disposition. Use ` +
        `${MEMORY_DISPOSITIONS.join(", ")}, or the wave it was folded into.`,
      );
    } else if (!known && !TARGET_WAVE.test(disposition)) {
      fail(
        `.specify/memory/${reg}:${i + 1}: disposition "${disposition}" is not in the ` +
        `closed set (${MEMORY_DISPOSITIONS.join(", ")}) and is not a legal target wave ` +
        `(W0..W8, or a qualified token such as spec001w1). Change it to one of those.`,
      );
    }
  }
  if (inFence) {
    fail(
      `.specify/memory/${reg}: the fenced block opened on line ${fenceOpenedAt} is never ` +
      `closed, so every line after it was skipped and any table below it went unread. ` +
      `Close the fence.`,
    );
  }
  if (!sawHeader) {
    fail(
      `.specify/memory/${reg}: no header row was found, so not one row in this ` +
      `register was validated. A register table needs a leading and a trailing ` +
      `pipe on every row, including its header. Add the header row.`,
    );
  } else if (rowsHere === 0 && !declaredEmpty) {
    fail(
      `.specify/memory/${reg}: this register has a header and not one data row. Rows are ` +
      `dispositioned in place rather than deleted, so an emptied register has lost history ` +
      `the triage rules count. Restore the rows. If it is genuinely empty, say so with a ` +
      `line reading exactly "${EMPTY_MARKER}".`,
    );
  } else if (rowsHere > 0 && declaredEmpty) {
    fail(
      `.specify/memory/${reg}: this register declares itself intentionally empty and has ` +
      `${rowsHere} rows. Left in place the marker would license a later truncation. Remove ` +
      `the "${EMPTY_MARKER}" line.`,
    );
  }
}

if (errors.length) {
  for (const e of errors) console.error(`error: ${e}`);
  console.error(`\ncheck-bindings: ${errors.length} error(s)`);
  process.exit(1);
}
console.log(
  `check-bindings: ${agents.size} agents, ${rows.length} binding rows, ` +
  `${registerRows} register rows, all consistent`,
);
