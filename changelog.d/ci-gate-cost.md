### Changed

- The pull-request gate no longer rebuilds a patched crosvm and a patched
  cloud-hypervisor from source on every run in order to check two of their
  command-line flags. The two outputs those checks need are carried between
  runs instead, at 30 MB, so the shard that set the gate's wall time stops
  doing so. A carried entry that no longer matches simply builds as before,
  because store paths change with the derivation.
- The compiler-derived API census runs as its own gate shard. It renders
  through a separately pinned toolchain into its own target directory and
  shares no artifacts with the workspace build it previously ran inside, so
  sequencing it there only lengthened the longest shard. `make test-rust`
  still runs every shard once in order, and `make test-rust-api-surface`
  matches the existing per-shard targets.
- The API census builds its dependency graph once rather than twice. Its
  public pass and its private-and-hidden pass differ only in flags that
  affect rendering, not in the dependencies they compile, so they now share
  one target directory. This halves both the time that pass costs on a cold
  tree and the disk it occupies.
- The privileged broker's default and layer1-bootstrap test passes no longer
  run a `cargo check` immediately before their `cargo test`. The two are
  distinct compilation modes that share no artifacts, so the check reported
  the same errors slightly earlier at the cost of running the compiler twice.
  Measured cold, that is 153 s against 89 s for an identical result. The
  fake-backends pass never had one.
