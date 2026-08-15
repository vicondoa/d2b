{ pkgs, bazel920, system }:

pkgs.runCommand "d2b-bazel-9.2.0-provider-smoke" {
  nativeBuildInputs = [ pkgs.coreutils ];
} ''
  set -euo pipefail

  test "${system}" = "x86_64-linux" -o "${system}" = "aarch64-linux"
  test "${bazel920.version}" = "9.2.0"
  test -x "${bazel920}/bin/bazel"
  # The official launcher is an ELF plus an appended ZIP and is intentionally
  # not patched. Verify its fetched bytes without executing it in the Nix
  # sandbox, whose FHS loader is not available to the upstream release.
  test "$(sha256sum "${bazel920.src}" | cut -d' ' -f1)" = \
    "${bazel920.passthru.upstreamSha256}"
  test "$(sha256sum "${bazel920}/bin/bazel" | cut -d' ' -f1)" = \
    "${bazel920.passthru.upstreamSha256}"
  mkdir -p "$out"
  printf '%s\n' "${system} Bazel ${bazel920.version}" > "$out/version"
''
