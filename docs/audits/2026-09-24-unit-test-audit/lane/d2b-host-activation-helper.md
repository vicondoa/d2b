# d2b-host-activation-helper - unit-test audit
tests: 2 · src files: 1
net: -0 tests, -0 lines

## Findings (biggest net first)
- gap: CLI validation error paths (`parse_args`/`parse_gid`, src/main.rs:63-105) - unknown argument, missing `--root`/`--target-gid`, empty `--legacy-gids`, gid out of range all end in `usage()` exit 64 with zero tests; a misspelled flag silently re-routes to usage instead of a clear error.
- gap: NUL-byte path/name rejection (`cstring_path`/`cstring_name`, src/main.rs:107-118) - InvalidInput error path untested; a hostile path would otherwise surface as an unlabeled failure.
- Nothing to cut. Ship. - both tests pin distinct real behavior against real filesystem state (tempdir + flock), neither duplicates the other nor any integration surface (crate has no `tests/` dir), and neither is plumbing.

## Keep
- `migrate_then_fail_closed_scan_reopens_root_for_full_walk` - pins fail-closed post-scan re-opening root and counting every legacy-gid entry (root itself + nested dir + file = 3) after a migration walk, i.e. scan counts dirs and root, not just files.
- `held_lock_fail_closed_runs_postscan_and_exits_nonzero` - pins lock-held + fail-closed path: `lock_is_held` probe succeeds and `run` performs the post-scan and exits nonzero on leftovers.
