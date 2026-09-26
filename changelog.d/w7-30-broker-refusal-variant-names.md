### Fixed

- The broker storage-contract refusal enum no longer trips the shared-prefix
  clippy style check: its variants drop the `Storage` prefix while the
  operator-visible refusal slugs are unchanged (`Display` still renders the
  same `storage-...` text every audit surface carries).
