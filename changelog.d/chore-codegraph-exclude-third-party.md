### Changed

- The codegraph index now excludes `third_party/` through a committed
  `codegraph.json`, so vendored agent-skill copies and other third-party
  trees no longer outrank first-party d2b code in graph queries. The file
  is honored by `codegraph init`, `index`, `sync`, and the watcher, so
  every clone picks the exclusion up without local configuration.
