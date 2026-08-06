---
name: d2b-caveman-compress
description: Compress only the checked-in d2b prompt corpus with protected-structure checks and a required semantic audit. Never invokes an external runtime or edits files outside the manifest.
user-invocable: true
---

# Governed prompt compression

Use this skill for the exact file set in
`scripts/copilot/prompt-corpus-manifest.json`. It is repository maintenance,
not a general document compressor.

## Preconditions and boundaries

1. Resolve the repository root and read the checked-in manifest.
2. Refuse every path outside it, including ADRs, source, feature artifacts,
   consumer docs, changelogs, and arbitrary memory files.
3. Snapshot allowed originals and comparisons under `.scratch/`; never create a
   freshness sidecar, digest chain, or backup beside a governed file.
4. Use only the current Copilot session. Never require Anthropic, Claude CLI,
   Python, an external install, network access, or repository-content upload.

## Rewrite contract

Compress natural-language prose in place while preserving meaning. Preserve
frontmatter, headings, fenced blocks and contents, inline code, links, URLs,
list hierarchy/count, table shape, numbers, versions, paths, flags, environment
variables, identifier-like tokens, normative operators, negations, exceptions,
commands, exact errors, and JSON/output-schema examples.

Do not merge or remove requirements. Keep causal order, refusal behavior,
initial-creation exceptions, ownership boundaries, validation commands, and
normal-prose boundaries. Do not require token reduction or grade style/brevity.

## Acceptance

Run `node scripts/copilot/prompt-corpus.mjs` after each rewrite. Review the
uncompressed snapshot beside the result and record the semantic audit in
transient `.scratch/` notes. Reject any protected fingerprint change or
unresolved semantic difference. The check proves structure and protected
literals, not semantic equivalence; side by side human review remains required.
