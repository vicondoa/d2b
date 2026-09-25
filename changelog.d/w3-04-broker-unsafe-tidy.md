### Changed

- Tightened the broker's unsafe-code audit surface in `sys.rs` path-safety and child-context helpers: every raw `libc`/syscall wrapper now carries a `// SAFETY:` invariant comment, so audited and un-audited blocks are distinguishable. No behavioral change.
