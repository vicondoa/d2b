# nix-unit cases migrated from tests/assertions-eval.sh (group E).
#
# The canonical eval-time assertion corpus: every consumer misconfig must be
# REJECTED with the expected reason. Reuses the EXACT case table
# (tests/unit/nix/eval-cases/assertions.nix) and the minimal `lib.evalModules`
# evaluator (tests/unit/nix/eval-cases/shared.nix, ~0.6 s/case — NOT a full
# nixosSystem), so this migration is on the fast path and does not add the
# heavy per-case nixosSystem eval cost.
#
# Two shapes (auto-derived from the batch result, mirroring the bash gate's
# Bucket A / Bucket B split):
#   * Bucket A (config.assertions FAILS, eval succeeds): assert the case's
#     `expectedSubstring` appears in the failing-assertion message list.
#     This PRESERVES the message-substring check (unlike a throw-only
#     `expectedError` migration).
#   * Bucket B (eval THROWS before config.assertions is computable — e.g.
#     platform or structural provider checks): `tryEval`
#     cannot capture the throw message, so assert only THAT eval is rejected
#     (`evalSucceeded == false`). The expected message is retained in
#     tests/unit/nix/eval-cases/assertions.nix for traceability. This bucket
#     includes the aarch64 platform-rejection coverage.
{ lib, nixpkgsFlake, d2bModule, ... }:

let
  batch = import ../eval-cases/assertions.nix {
    nixpkgs = nixpkgsFlake;
    inherit d2bModule;
  };

  bucketB = [
    "audio-requires-audio-provider-binding"
    "graphics-without-wayland-user"
    "realm-name-invalid"
    "wayland-requires-display-provider-binding"
    "workload-name-invalid"
  ];

  mkCase = name: result:
    if builtins.elem name bucketB then
      # Bucket B — must be rejected via a throw.
      {
        expr = result.evalSucceeded;
        expected = false;
      }
    else
      # Bucket A — eval succeeds, but a failing assertion message carries
      # the expected substring.
      {
        expr =
          result.evalSucceeded
          && lib.any (m: lib.hasInfix result.expectedSubstring m) result.failingMessages;
        expected = true;
      };
in
lib.mapAttrs'
  (name: result: lib.nameValuePair "assertions/${name}" (mkCase name result))
  batch
