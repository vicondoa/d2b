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
#     (`nix-unit tests/unit/nix/default.nix --eval-store auto`), which reads
#     the exact same `{ expr; expected; }` shape.
#
# `ctx` carries everything any case might need; cases destructure only what
# they use:
#   { lib, pkgs, system, flakeRoot, nl, mkEval }
{ lib, pkgs, system, flakeRoot, nl, mkEval ? null, nixpkgsFlake ? null, nixlingModule ? null }:

let
  ctx = { inherit lib pkgs system flakeRoot nl mkEval nixpkgsFlake nixlingModule; };

  # Auto-discover every case module under ./cases so parallel W2 migration
  # units can each DROP a new `cases/<gate>.nix` file without editing this
  # shared aggregator (mirrors the W1 parallel-unit protocol). A unit adds
  # its case file + its migration-state.d row + deletes its legacy `.sh`;
  # it never touches default.nix.
  casesDir = ./cases;
  caseFiles = map (n: casesDir + "/${n}")
    (lib.filter (n: lib.hasSuffix ".nix" n)
      (lib.attrNames (builtins.readDir casesDir)));

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
