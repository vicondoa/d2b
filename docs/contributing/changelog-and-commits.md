# Changelog and commit conventions

Every pull request that changes code must ship release notes. The CI gate
accepts either an entry in `CHANGELOG.md` or a fragment under `changelog.d/`.

## Changelog

Use [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Add entries
under `## [Unreleased]`; when releasing, rename that section to
`## [X.Y.Z] - YYYY-MM-DD`.

When branches overlap, do not edit `CHANGELOG.md`. Write one
`changelog.d/<branch-name>.md` fragment with the same `### <Section>` headings.
The integrator folds fragments with:

```bash
make changelog-fold
```

The fold collates sections in Keep a Changelog order, leaves released versions
untouched, and deletes consumed fragments. Unknown or repeated headings, empty
sections, or content outside a section fail closed. See
[`changelog.d/README.md`](../../changelog.d/README.md).

Follow semver. The version in `CHANGELOG.md` is the single source of truth.
Merging to `v3` with a new version header triggers the release workflow.

## Commit conventions

- Use a short, imperative subject prefixed with the touched area, such as
  `net: fix 10-eth-dhcp neutralization`.
- Wrap the body at about 72 columns and explain why; the diff shows what.
- Keep one logical change per commit. Mechanical reformatting or renames get
  their own commit.
- Do not include AI, tool, or model attribution, or a `Co-authored-by`
  trailer for an AI tool unless explicitly requested.
- GPG signing is not used.
- Use only the ASCII hyphen `-` for dashes in subjects and bodies.

## Release hygiene

Released sections are coherent, consumer-facing summaries grouped under the
standard Keep a Changelog headings: `Added`, `Changed`, `Fixed`, `Deprecated`,
`Removed`, and `Security`. Do not include internal planning identifiers in
released prose or changelog fragments.
