### Fixed

- A Device-owned worker's per-Guest runtime socket directory
  (`/run/d2b/vms/<guest>`) is now created by the broker's socket grant, with
  the mode and ownership the trusted `path:vm-run:<guest>` storage row
  declares, immediately before the grant opens it to the worker's principal.
  A zone-native Guest has no static tmpfiles rule for its own directory, so
  the row was a declaration nothing acted on: the grant walked to an absent
  leaf, `setfacl` returned `Ok(None)` for it, and the launch proceeded with no
  ACL anywhere - the worker then failed with an opaque error of its own.
- The runner-tree ACL grant fails closed on an absent leaf only where the
  grant OWNS that leaf. The device worker's per-Guest runtime directory is
  created from the trusted `path:vm-run:<guest>` row on the line above, so an
  absent one there is a grant that could not be applied and the launch is
  refused - which is the silent `setfacl` no-op this replaces, since
  `setfacl` returns `Ok(None)` for a path that is not there. A directory the
  broker cannot create or posture (a missing `/run/d2b/vms` parent, which the
  host's tmpfiles rule owns) is refused too, with a typed, path-free slug in
  the audit record, and so is a typed Device launch whose storage contract
  names no posture row for that Guest: the broker will not invent an
  identity for a directory no trusted row describes. The serving worker's
  private socket directory and the cloud-hypervisor state directory are NOT
  the broker's to create - the daemon realizes the first while it composes
  the launch argv, cloud-hypervisor creates the second - so an absent leaf
  there is a provisioning-order race and is skipped rather than refused the
  launch. The absent-ancestor refusal is unchanged for every caller: a
  partially-open chain still never gets a partial grant.
- The daemon no longer re-stamps the per-Guest runtime directory it
  realizes for a serving worker. That directory is shared with the Device
  workers for the same Guest, and every principal reaches it through its own
  named ACL entry; POSIX rewrites a file's ACL mask from its group bits on
  every `chmod`, so a `0700` stamp over a directory that already existed
  dropped the mask to `---` and turned every one of those entries into
  `#effective:---`. The Device worker then could not bind its own socket
  there, exited, and was relaunched into the same revoked state while the
  stamp landed again - a loop the TPM endpoints never came out of, so the
  Device never attached and the Guest's Volumes never became Ready. Only a
  directory that call created is stamped now.
