### Added

- omp sessions in this repository now get the codegraph code-intelligence
  MCP server through a committed `.omp/mcp.json` (pinned
  `@colbymchenry/codegraph@1.6.0` launched via `npx`), so any clone's omp
  picks up graph-backed exploration and impact-analysis tools without
  local configuration.
- The `d2b` devShell now carries `nodejs` and installs the same pinned
  codegraph npm package into `~/.cache/d2b-npm-global` for manual CLI
  use. The npm spec is parsed from `.omp/mcp.json` at evaluation time, so
  the shell install and the MCP wiring cannot drift; a missing or
  reshaped entry fails shell evaluation with an explicit message instead
  of skipping silently.
- `/.codegraph/`, the local index built by `codegraph init`, is now
  gitignored and never committed.
- A cloner running omp outside `nix develop` on a host without node must
  install node (for example `nix profile install nixpkgs#nodejs`) or enter
  the devShell before the `npx`-based server can launch.
