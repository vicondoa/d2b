---
name: d2b-caveman-compress
description: Compress only the checked-in d2b prompt corpus with protected-structure checks and a required semantic audit. Never invokes an external runtime or edits files outside the manifest.
user-invocable: true
---

# Governed prompt compression

Use this skill for the exact file set in
`scripts/copilot/prompt-corpus-manifest.json`. It is a repository maintenance
operation, not a general document compressor.

## Preconditions and boundaries

1. Resolve the repository root and read the checked-in manifest.
2. Refuse every path not in the manifest, including ADRs, source, feature
   artifacts, consumer documentation, changelogs, and arbitrary memory files.
3. Snapshot allowed originals and comparison output under `.scratch/`; never
   create a freshness sidecar, digest chain, or backup beside a governed file.
4. Use the current Copilot session only. Never require Anthropic, Claude CLI,
   Python, an external install, network access, or repository-content upload.

## Rewrite contract

Compress natural-language prose in place while preserving semantics and
professional meaning. Preserve frontmatter, headings, fenced blocks and their
contents, inline code, links, URLs, list hierarchy and count, table shape,
numbers, versions, paths, flags, environment variables, identifier-like
tokens, normative operators, negations, exceptions, commands, exact errors,
and exact JSON or output-schema examples.

Do not merge or remove requirements. Keep causal order, refusal behavior,
initial-creation exceptions, ownership boundaries, validation commands, and
normal-prose boundaries. Do not require a token reduction and do not grade
style or brevity.

## Acceptance

Run `node scripts/copilot/prompt-corpus.mjs` after each rewrite. Review the
uncompressed snapshot side by side with the result and record the semantic
audit in transient `.scratch/` notes. Reject the rewrite on any protected
fingerprint change or unresolved semantic difference. A successful check proves
structure and protected literals, not semantic equivalence; human side-by-side
review remains required.

