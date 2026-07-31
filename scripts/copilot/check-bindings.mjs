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

import { existsSync, readFileSync, readdirSync } from "node:fs";
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

const errors = [];
const warnings = [];
const fail = (m) => errors.push(m);
const warn = (m) => warnings.push(m);

function readPolicy() {
  if (!existsSync(modelRs)) {
    warn(`cannot read policy constants: ${modelRs} not found`);
    return null;
  }
  const src = readFileSync(modelRs, "utf8");
  const pick = (name) => {
    const m = src.match(new RegExp(`${name}:\\s*&str\\s*=\\s*"([^"]+)"`));
    return m ? m[1] : null;
  };
  const roles = [];
  const rolesBlock = src.match(/PANEL_ROLES[^=]*=\s*\[([\s\S]*?)\];/);
  if (rolesBlock) {
    for (const m of rolesBlock[1].matchAll(/PanelRole::(\w+)/g)) {
      roles.push(m[1].replace(/([a-z0-9])([A-Z])/g, "$1-$2").toLowerCase());
    }
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
    fail(`${label}: no YAML frontmatter`);
    return null;
  }
  const end = text.indexOf("\n---\n", 3);
  if (end === -1) {
    fail(`${label}: unterminated frontmatter`);
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
  fail(`${agentsDir} does not exist`);
} else {
  for (const file of readdirSync(agentsDir).sort()) {
    if (!file.endsWith(".agent.md")) continue;
    const name = file.slice(0, -".agent.md".length);
    const text = readFileSync(join(agentsDir, file), "utf8");
    const fm = parseFrontmatter(text, file);
    if (!fm) continue;

    if (fm.name !== name) {
      fail(`${file}: frontmatter name "${fm.name}" does not match the file basename "${name}"`);
    }
    if (!fm.description) fail(`${file}: description is required for dispatch selection`);

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
        `would run on the architect's model and be attested as Gemini.`,
      );
    } else if (!CAPABILITIES[fm.model]) {
      warn(`${file}: model "${fm.model}" is not in the known capability table; effort cannot be checked`);
    }
    if (name.startsWith("panel-")) {
      const tools = fm.tools ?? "";
      if (/\b(bash|edit|create|write|task|sql)\b/.test(tools)) {
        fail(
          `${file}: panel agents are read-only by construction. "tools:" must not grant ` +
          `${tools}. Reviewers read staged diffs; granting shell also puts ten lanes on ` +
          `the shared Nix store and the heavy-gate semaphore.`,
        );
      }
      if (!/\bview\b/.test(tools)) {
        fail(`${file}: panel agent needs "view" to read the staged diffs`);
      }
    }
    agents.set(name, { file, model: fm.model, tools: fm.tools ?? "" });
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
      `context_tier; an unbound agent will silently run at the model default effort.`,
    );
  }
}

const policy = readPolicy();

for (const r of rows) {
  const a = agents.get(r.agent);
  if (a.model && r.model !== a.model) {
    fail(
      `${r.skill}/SKILL.md: row for "${r.agent}" pins model "${r.model}" but ` +
      `${a.file} frontmatter pins "${a.model}". These must agree.`,
    );
  }
  const caps = CAPABILITIES[r.model];
  if (!caps) {
    warn(`${r.skill}/SKILL.md: unknown model "${r.model}" for "${r.agent}"`);
    continue;
  }
  if (!caps.efforts.includes(r.effort)) {
    fail(
      `${r.skill}/SKILL.md: reasoning_effort "${r.effort}" is not valid for "${r.model}" ` +
      `(valid: ${caps.efforts.join(", ")}). The observed failure mode for an invalid ` +
      `effort is a silent downgrade, not an error.`,
    );
  }
  if (!caps.tiers.includes(r.tier)) {
    fail(`${r.skill}/SKILL.md: context_tier "${r.tier}" is not valid for "${r.model}" (valid: ${caps.tiers.join(", ")})`);
  }
  if (policy && r.agent.startsWith("panel-")) {
    if (policy.model && r.model !== policy.model) {
      fail(
        `${r.skill}/SKILL.md: panel row "${r.agent}" pins model "${r.model}" but ` +
        `PANEL_MODEL_POLICY is "${policy.model}". panel-attest would reject those records.`,
      );
    }
    if (policy.effort && r.effort !== policy.effort) {
      fail(
        `${r.skill}/SKILL.md: panel row "${r.agent}" pins effort "${r.effort}" but ` +
        `PANEL_REASONING_EFFORT_POLICY is "${policy.effort}".`,
      );
    }
  }
}

// Every roster seat must have an agent.
if (policy && policy.roles.length) {
  for (const role of policy.roles) {
    if (!agents.has(`panel-${role}`)) {
      fail(`PANEL_ROLES names seat "${role}" but there is no .github/agents/panel-${role}.agent.md`);
    }
  }
  for (const name of agents.keys()) {
    if (name.startsWith("panel-") && !policy.roles.includes(name.slice("panel-".length))) {
      fail(`agent "${name}" is not a seat in PANEL_ROLES; the roster is closed`);
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
        `this file would silently govern nothing.`,
      );
    }
  }
}

// spec-kit coexistence. `specify init` REPLACES installed_integrations rather
// than appending, so re-running it for Copilot silently drops the opencode
// install the in-flight program uses. It also rewrites shared files under
// .specify/scripts and .specify/templates, reintroducing banned dash
// codepoints into tracked files; the tier0 dash scan catches that one, but
// nothing catches this one.
const integrationJson = join(root, ".specify", "integration.json");
if (existsSync(integrationJson)) {
  let state = null;
  try {
    state = JSON.parse(readFileSync(integrationJson, "utf8"));
  } catch (e) {
    fail(`.specify/integration.json is not valid JSON: ${e.message}`);
  }
  if (state) {
    const installed = state.installed_integrations ?? [];
    for (const required of ["copilot", "opencode"]) {
      if (!installed.includes(required)) {
        fail(
          `.specify/integration.json no longer lists "${required}" in ` +
          `installed_integrations. Both must remain until the cutover: "specify init" ` +
          `replaces this array rather than appending to it, so this is the expected ` +
          `shape of an accidental re-init.`,
        );
      }
    }
    for (const key of ["integration", "default_integration"]) {
      if (state[key] !== "opencode") {
        warn(
          `.specify/integration.json ${key} is "${state[key]}", not "opencode". That ` +
          `only selects the cosmetic invoke separator, but the standing overlap rule is ` +
          `that the old path wins where the two disagree.`,
        );
      }
    }
  }
}

for (const w of warnings) console.warn(`warning: ${w}`);
if (errors.length) {
  for (const e of errors) console.error(`error: ${e}`);
  console.error(`\ncheck-bindings: ${errors.length} error(s)`);
  process.exit(1);
}
console.log(`check-bindings: ${agents.size} agents, ${rows.length} binding rows, all consistent`);
