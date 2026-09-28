### Removed

- The broker's swtpm NVRAM tamper guard, at the user's explicit direction and
  against the assistant's recommendation. `PrepareSwtpmDir` and its
  `packages/d2b-broker/src/ops/swtpm_dir.rs` module are gone, and with them the
  identity-bound marker (`/var/lib/d2b/swtpm-markers/<vm>`, root:root 0600, a
  regular file placed outside the directory it guards), the per-spawn
  verification of that marker against the live directory's owner, mode, and
  `st_dev`/`st_ino`, and the fail-closed fence that refused to spawn a swtpm
  worker on any mismatch. The safety property given up is real: a process that
  owns the per-VM swtpm state directory can now replace it, or swap its
  contents for a directory of its own, and the broker will not notice or
  refuse. Because the TPM 2.0 Endorsement Key is unrecoverable by design, a
  swapped NVRAM silently invalidates the Guest's attested identity rather than
  failing loudly. The marker tree is not recreated, wiped, or chown-touched by
  anything in its place.
- The state-directory access grants are NOT part of this removal and are
  unchanged: `grant_swtpm_state_dir_traversal` (the traverse-ACL fix for the
  0700 TPM state root, whose mask POSIX rewrites from the group bits on every
  chmod) and `DeviceWorkerSocketGrant::apply` (the per-Guest runtime directory
  creation and posturing) both survive, with their regression tests. The
  trusted-identity and path-derivation those grants read moved to
  `packages/d2b-broker/src/ops/swtpm_identity.rs`; the sandbox, the seccomp
  policy, the `writable_paths` boundary, and the ownership check are untouched.
