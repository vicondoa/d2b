# d2b public option schema.
#
# Isolation and workload authoring live exclusively under
# `d2b.zones.<zone>.resources.<name>`. Host admission and deployment
# defaults remain in the focused site, host, daemon, and observability
# option modules below. Removed pre-Zone option paths are intentionally not
# declared here, so NixOS reports them through its ordinary unknown-option
# behavior.
{ lib, ... }:

{
  imports = [
    ./options-site.nix
    ./options-host-sccache.nix
    ./options-host.nix
    ./options-daemon.nix
  ];
}
