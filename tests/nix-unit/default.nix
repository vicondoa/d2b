# nix-unit test corpus entrypoint (W2).
#
# Each module under ./cases is a function `ctx: { <name> = case; }` where a
# `case` follows the upstream nix-unit convention:
#
#   { expr = <value>; expected = <value>; }          # value assertion
#   { expr = <thunk>; expectedError = { ... }; }      # throw assertion
#
# Case names are slash-namespaced by their originating gate
# (e.g. "volume-mounts/serial-null-defaults") so a failure points straight
# at the retired bash gate it replaced.
#
# Two consumers share this one corpus:
#   * flake.checks.<sys>.nix-unit  — the hermetic CI gate (a pure-eval
#     comparison runner; NO recursive-nix / IFD), wired in flake.nix.
#   * the upstream `nix-unit` CLI for local iteration
#     (`nix-unit tests/nix-unit/default.nix --eval-store auto`), which reads
#     the exact same `{ expr; expected; }` shape.
#
# `ctx` carries everything any case might need; cases destructure only what
# they use:
#   { lib, pkgs, system, flakeRoot, nl, mkEval }
{ lib, pkgs, system, flakeRoot, nl, mkEval ? null }:

let
  ctx = { inherit lib pkgs system flakeRoot nl mkEval; };

  caseFiles = [
    ./cases/volume-mounts.nix
  ];

  merge = acc: f:
    let cases = import f ctx;
    in acc // (
      # Fail loudly on a duplicate case name across files rather than
      # silently dropping one with `//`.
      let dup = lib.attrNames (lib.intersectAttrs cases acc);
      in if dup == [ ] then cases
      else throw "nix-unit: duplicate case name(s) across corpus files: ${lib.concatStringsSep ", " dup}"
    );
in
builtins.foldl' merge { } caseFiles
