{ lib, ... }:

{
  options.d2b.site.hostSccache.enable = lib.mkOption {
    type = lib.types.bool;
    default = false;
    example = true;
    description = ''
      Provision the fixed multi-user host sccache at
      /var/cache/d2b-sccache. When enabled, d2b adds the cache to the
      Nix daemon's global extra-sandbox-paths and creates it as
      root:nixbld with setgid mode 2770 so Nix build users can share
      compiler outputs without exposing a caller-owned home directory.

      This is opt-in because it changes host-wide Nix daemon policy. It remains
      available for Nix source builds that use the d2b host-tool derivations;
      the Bazel-backed host-integration lane supplies its tools separately and
      does not require this option.
    '';
  };
}
