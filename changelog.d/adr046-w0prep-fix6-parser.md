### Changed

- The ADR 0046 envelope and spec-literal lints now build their document model
  with real parsers instead of a hand-written multi-language reader: JSON is
  parsed by `serde_json`, YAML by `serde_yaml_ng`, and Nix by `rnix`. Each
  parse has an explicit error channel and every caller treats a parse error as
  fail closed, scoped to the check's genuine authoring trigger, so a block a
  parser cannot model is reported rather than silently skipped. The previous
  hand-written parser mis-modelled valid syntax (it discarded `rec { ... }`
  attrsets, left JSON `\uXXXX` key escapes undecoded, and mishandled YAML
  anchors, tags, and merge keys), which let real violations disappear.
- Envelope classification is now per document rather than per fence. Each
  `apiVersion` document resolves to exactly one of a live resource envelope, a
  `resourceType` bundle envelope, or an explicit unrecognised case that fails
  closed, so one recognised envelope can no longer mask an unrecognised sibling
  in the same fenced block, and a document carrying neither `type` nor
  `resourceType` is flagged rather than classified as nothing.
- The D116 negative-example exemption now binds to exactly one parsed resource
  rather than a whole fenced block. The marker comment is read from the parsed
  document and suppresses only the single resource map that lexically contains
  it, so an unmarked, genuinely violating resource beside the marked teaching
  example in the same fence is still reported. The exemption remains pinned to
  the one spec file and a single marker occurrence, and fails closed otherwise.
- The universal-status lint now scans Nix fences in addition to YAML and JSON,
  decodes JSON `\uXXXX` key escapes so an escaped `type` key is classified as a
  live envelope, folds YAML `<<` merge keys before judging the assembled status,
  and honours elision only as `status: ...` or a direct `...` marker key, never
  as a `...` value on some other status field such as `conditions`.
- The D103 datetime, D104 ResourceType, and D108 retry-scalar lints now inspect
  the complete parsed scalar in value position across YAML, JSON, and Nix rather
  than matching a line shape. A key and value split across lines, a
  punctuation-suffixed timestamp such as `2026-07-22T00:00:00.000Z_junk`, an
  over-qualified `type: "acme.d2bus.org.Widget.Type"`, a quoted JSON
  `"retryAfterMs"` key, and a non-finite `retryAfterMs: NaN` value are all now
  rejected where the previous line-regex passes let them through.

### Fixed

- The ADR 0046 policy lints now emit repository-relative paths in every
  diagnostic and panic message. A read failure or violation report no longer
  prints the absolute checkout root or a username-bearing path into CI logs;
  each surface renders a path under the repository root, falling back to the
  bare file name rather than an absolute path.
