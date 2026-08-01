### Changed

- The pull-request gate no longer rebuilds a patched crosvm and a patched
  cloud-hypervisor from source on every run in order to check two of their
  command-line flags. The two outputs those checks need are carried between
  runs instead, at 30 MB, so the shard that set the gate's wall time stops
  doing so. Measured on the gate: that shard falls from 1010 s to 33 s, with
  no build of either package. A carried entry that no longer matches simply
  builds as before, because store paths change with the derivation.
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
- The gate's Nix-store cache entry can now be replaced when its key changes.
  The job was already configured to purge the entry it supersedes, but the
  workflow granted no permission to delete one, so the purge failed after the
  replacement had been saved and the superseded entry stayed resident forever.
  Two such entries were resident at roughly 1.25 GiB each against a hard
  repository-wide budget, and overrunning that budget evicts entries other
  jobs depend on. The permission is granted to that job alone, which is the
  only one that deletes anything.
- The resource API's external capability seal reuses its fixture build between
  runs, keyed on the compiler that produced it, rather than rebuilding roughly
  a gigabyte of dependencies every time. Measured locally at 40 s against 2 s.
  The seal's proof is unchanged: it still demonstrates that the crate compiled
  under forced `cfg(test)`, by discarding that one crate's fingerprints while
  its dependencies stay warm, and it now discards the marker recording that
  compile at the start of every run - so if the forcing were ever to stop
  working the seal fails rather than passing without proof.
