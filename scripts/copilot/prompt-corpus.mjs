#!/usr/bin/env node
// Govern the exact prompt corpus without grading prose style or token count.
//
// `node scripts/copilot/prompt-corpus.mjs` validates the checked-in manifest.
// `node scripts/copilot/prompt-corpus.mjs --capture` refreshes it before the
// governed prose is compressed. Capture is an intentional maintenance action;
// normal checks are fail-closed and never rewrite the manifest.

import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const manifestPath = join(root, "scripts", "copilot", "prompt-corpus-manifest.json");

const ROOT_FILES = [
  "AGENTS.md",
  "tests/AGENTS.md",
  "labs/venus-vulkan-video/AGENTS.md",
];

function sortedMarkdownFiles(directory, suffix) {
  if (!existsSync(directory)) return [];
  return readdirSync(directory)
    .filter((name) => name.endsWith(suffix))
    .sort()
    .map((name) => relative(root, join(directory, name)).replaceAll("\\", "/"));
}

function expectedCorpus() {
  const docs = sortedMarkdownFiles(join(root, "docs", "contributing"), ".md");
  const agents = sortedMarkdownFiles(join(root, ".github", "agents"), ".agent.md");
  const skills = readdirSync(join(root, ".github", "skills"))
    .filter((name) => name.startsWith("d2b-"))
    .sort()
    .filter((name) => existsSync(join(root, ".github", "skills", name, "SKILL.md")))
    .map((name) => `.github/skills/${name}/SKILL.md`);
  return [...ROOT_FILES, ...docs, ...agents, ...skills];
}

function digest(value) {
  return createHash("sha256").update(value).digest("hex");
}

function fingerprint(value) {
  return { bytes: Buffer.byteLength(value, "utf8"), sha256: digest(value) };
}

function lineRecords(text) {
  const records = [];
  const re = /[^\n]*(?:\n|$)/g;
  for (let match = re.exec(text); match && match[0] !== ""; match = re.exec(text)) {
    records.push({
      text: match[0].endsWith("\n") ? match[0].slice(0, -1) : match[0],
      start: match.index,
      end: match.index + match[0].length,
    });
  }
  return records;
}

function fencedParts(text) {
  const lines = lineRecords(text);
  const blocks = [];
  const masked = [...text];
  let opening = null;
  for (const line of lines) {
    const match = line.text.match(/^\s*(`{3,}|~{3,})(.*)$/);
    if (opening === null && match) {
      opening = { start: line.start, fence: match[1], info: match[2].trim() };
    } else if (opening !== null && match && match[1][0] === opening.fence[0]) {
      const end = line.end;
      const value = text.slice(opening.start, end);
      blocks.push({
        info: opening.info,
        fingerprint: fingerprint(value),
        value,
      });
      opening = null;
    }
    if (opening !== null || (match && blocks.length > 0 && blocks.at(-1).fingerprint)) {
      for (let i = line.start; i < line.end; i += 1) {
        if (text[i] !== "\n") masked[i] = " ";
      }
    }
  }
  if (opening !== null) {
    const value = text.slice(opening.start);
    blocks.push({
      info: opening.info,
      fingerprint: fingerprint(value),
      value,
    });
    for (let i = opening.start; i < text.length; i += 1) {
      if (text[i] !== "\n") masked[i] = " ";
    }
  }
  return {
    blocks,
    masked: masked.join(""),
  };
}

function sequence(values) {
  return values.map((value) => fingerprint(value));
}

function frontmatter(text) {
  if (!text.startsWith("---\n")) return [];
  const end = text.indexOf("\n---\n", 4);
  if (end < 0) return [];
  return [text.slice(0, end + 5)];
}

function headings(masked) {
  return sequence(
    lineRecords(masked)
      .map((line) => line.text)
      .filter((line) => /^#{1,6}[ \t]+/.test(line).valueOf()),
  );
}

function inlineCode(masked) {
  const values = [];
  const re = /(`+)([\s\S]*?)\1/g;
  for (let match = re.exec(masked); match; match = re.exec(masked)) {
    values.push(match[0]);
  }
  return sequence(values);
}

function linksAndUrls(masked) {
  const values = [];
  const re = /\[[^\]\n]+\]\([^\)\n]+\)|https?:\/\/[^\s<>\)]+|www\.[^\s<>\)]+/g;
  for (let match = re.exec(masked); match; match = re.exec(masked)) {
    values.push(match[0]);
  }
  return sequence(values);
}

function lists(masked) {
  return lineRecords(masked)
    .map((line) => {
      const match = line.text.match(/^(\s*)([-+*]|\d+[.)])\s+/);
      if (!match) return null;
      return {
        indent: match[1].replaceAll("\t", "    ").length,
        marker: match[2],
        ordered: /^\d/.test(match[2]),
      };
    })
    .filter(Boolean);
}

function tableShape(masked) {
  return lineRecords(masked)
    .map((line) => line.text)
    .filter((line) => {
      const trimmed = line.trim();
      return trimmed.startsWith("|") && trimmed.includes("|");
    })
    .map((line) => {
      let pipes = 0;
      let escaped = false;
      for (const char of line) {
        if (escaped) {
          escaped = false;
        } else if (char === "\\") {
          escaped = true;
        } else if (char === "|") {
          pipes += 1;
        }
      }
      const trimmed = line.trim();
      return {
        columns: Math.max(0, pipes + 1 - (trimmed.startsWith("|") ? 1 : 0) -
          (trimmed.endsWith("|") ? 1 : 0)),
        leadingPipe: trimmed.startsWith("|"),
        trailingPipe: trimmed.endsWith("|"),
        separator: /^\|?\s*:?-{3,}:?(\s*\|\s*:?-{3,}:?)+\s*\|?$/.test(trimmed),
      };
    });
}

function literals(masked) {
  const values = [];
  const re = /(\$[A-Z][A-Z0-9_]*)|(--?[A-Za-z][A-Za-z0-9_-]*)|(\bv?\d+(?:\.\d+){1,}\b)|((?:\.{0,2}\/|\/)[A-Za-z0-9_./-]+)|(\b(?:[A-Z]{2,}[A-Z0-9_-]*|[a-z][A-Za-z0-9]*_[A-Za-z0-9_]+|[a-z][a-z0-9]*-[a-z0-9_-]+)\b)|(\b\d+(?:\.\d+)?\b)/g;
  for (let match = re.exec(masked); match; match = re.exec(masked)) {
    const kind = match[1] ? "env" : match[2] ? "flag" : match[3] ? "version" :
      match[4] ? "path" : match[5] ? "identifier" : "number";
    values.push({ kind, ...fingerprint(match[0]) });
  }
  return values;
}

function normative(masked) {
  const values = [];
  const re = /\b(MUST\s+NOT|SHOULD\s+NOT|FAILS?\s+CLOSED|FAIL\s+CLOSED|MUST|SHOULD|NEVER|ONLY|NOT|EXCEPT|WITHOUT|NO|MAY|REQUIRED|REFUSE|REJECT)\b/g;
  for (let match = re.exec(masked); match; match = re.exec(masked)) {
    values.push(match[0]);
  }
  return sequence(values);
}

function jsonExamples(blocks) {
  return blocks
    .filter((block) => /^(json|jsonc)(?:\s|$)/i.test(block.info))
    .map((block) => block.fingerprint);
}

function outputExamples(text) {
  return sequence(
    [...text.matchAll(/Return exactly one JSON object and nothing else:\n\n```json\n[\s\S]*?\n```/g)]
      .map((match) => match[0]),
  );
}

function communicationBlocks(text) {
  const values = [];
  const re = /<!-- BEGIN D2B-CAVEMAN-COMMUNICATION -->[\s\S]*?<!-- END D2B-CAVEMAN-COMMUNICATION -->/g;
  for (let match = re.exec(text); match; match = re.exec(text)) values.push(match[0]);
  return sequence(values);
}

function fingerprints(text) {
  const { blocks, masked } = fencedParts(text);
  return {
    frontmatter: frontmatter(text).map(fingerprint),
    headings: headings(masked),
    fencedBlocks: blocks.map(({ info, fingerprint: value }) => ({ info, ...value })),
    inlineCode: inlineCode(masked),
    linksAndUrls: linksAndUrls(masked),
    listHierarchy: lists(masked),
    tableShape: tableShape(masked),
    literals: literals(masked),
    normative: normative(masked),
    jsonExamples: jsonExamples(blocks),
    outputExamples: outputExamples(text),
    communicationBlocks: communicationBlocks(text),
  };
}

function expectedManifest() {
  return {
    schemaVersion: 1,
    membership: expectedCorpus(),
    files: expectedCorpus().map((path) => ({
      path,
      fingerprints: fingerprints(readFileSync(join(root, path), "utf8")),
    })),
  };
}

function compareExpectedMembership(manifest) {
  const expected = expectedCorpus();
  if (!Array.isArray(manifest.membership) ||
      JSON.stringify(manifest.membership) !== JSON.stringify(expected)) {
    console.error("prompt corpus membership differs from the checked-in manifest.");
    console.error(`expected ${expected.length} files, manifest has ${manifest.membership?.length ?? "no membership"}.`);
    return false;
  }
  if (!Array.isArray(manifest.files) ||
      JSON.stringify(manifest.files.map((file) => file.path)) !== JSON.stringify(expected)) {
    console.error("prompt corpus file records do not exactly match membership.");
    return false;
  }
  if (expected.length !== 32 ||
      expected.filter((path) => path === "AGENTS.md" || path.endsWith("/AGENTS.md")).length !== 3 ||
      expected.filter((path) => path.startsWith("docs/contributing/")).length !== 8 ||
      expected.filter((path) => path.startsWith(".github/agents/")).length !== 13 ||
      expected.filter((path) => path.startsWith(".github/skills/d2b-")).length !== 8) {
    console.error("prompt corpus dynamic enumeration is not the approved 32-file shape.");
    return false;
  }
  return true;
}

function validate() {
  if (!existsSync(manifestPath)) {
    console.error(`missing prompt corpus manifest: ${manifestPath}`);
    return 1;
  }
  let manifest;
  try {
    manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
  } catch (e) {
    console.error(`prompt corpus manifest is not valid JSON: ${e.message}`);
    return 1;
  }
  if (manifest.schemaVersion !== 1 || !compareExpectedMembership(manifest)) return 1;
  let failures = 0;
  for (const file of manifest.files) {
    const path = join(root, file.path);
    if (!existsSync(path) || !statSync(path).isFile()) {
      console.error(`missing governed prompt file: ${file.path}`);
      failures += 1;
      continue;
    }
    const actual = fingerprints(readFileSync(path, "utf8"));
    if (JSON.stringify(actual) !== JSON.stringify(file.fingerprints)) {
      console.error(`protected prompt fingerprint changed: ${file.path}`);
      failures += 1;
    }
  }
  if (failures) {
    console.error(`prompt corpus: ${failures} file(s) failed protected-fingerprint checks`);
    return 1;
  }
  console.log(`prompt corpus: ${manifest.files.length} governed files, protected fingerprints intact`);
  return 0;
}

function capture() {
  const manifest = expectedManifest();
  if (!compareExpectedMembership(manifest)) return 1;
  writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
  console.log(`prompt corpus: captured ${manifest.files.length} files`);
  return 0;
}

process.exitCode = process.argv.includes("--capture") ? capture() : validate();
