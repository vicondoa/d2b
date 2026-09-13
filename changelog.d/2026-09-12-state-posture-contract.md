### Added

- The host posture for state farms and shared directories is a declared
  contract, `packages/d2b-broker/src/ops/state-posture-contract.json`: per path
  level it names the creator, owner/group/mode, POSIX ACL entries (and who
  applies them), and the rights each host principal needs or is refused.
  One file feeds all three consumers - the broker posture code embeds and
  applies the `guest-store-view` rows, `nixos-modules/host-daemon.nix` derives
  the shared-run-dir and state-root tmpfiles lines from the declared rows, and
  the declarations are validated live.
- `tests/host-integration/state-posture-contract.nix` boots the Zone-native
  Cloud Hypervisor Guest recipe and asserts the live host against every
  declared level: owner/group/mode/ACL equality, and the allowed AND denied
  operations per principal (root, `d2bd`, a `d2b`-group launcher, and an
  unrelated user) for the per-Guest state chain, the per-Guest directory, the
  store-view farm, `/run/d2b`, and the state root. It fails if a level's
  group/mode/ACL stops matching the declaration, or if the daemon loses the
  search-only reach it has on levels it does not own.

### Changed

- The store-view posture code no longer restates its per-level modes: it reads
  the `guest-store-view` rows from the declaration and fails closed on a row
  it cannot apply. The declaration also names the anchor-open rule an anchored
  walk must obey - anchor components open `O_PATH`, only an owned leaf opens
  `O_RDONLY` - so read-on-traversal-only cannot be reintroduced quietly.
