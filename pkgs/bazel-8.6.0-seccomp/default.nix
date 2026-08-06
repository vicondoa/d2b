{ pkgs }:

let
  version = "8.6.0";
  source = pkgs.fetchzip {
    url = "https://github.com/bazelbuild/bazel/releases/download/${version}/bazel-${version}-dist.zip";
    hash = "sha256-W22eB0IzHNZe3xaF8AZOkUTDCic3NXkypdqSDY61Su0=";
    stripRoot = false;
  };
  policy = ./seccomp-policy.json;
  sandboxPatch = ./linux-sandbox-seccomp.patch;
in
pkgs.bazel_8.overrideAttrs (old: {
  pname = "bazel-${version}-seccomp";
  inherit version;
  src = source;
  patches = (old.patches or [ ]) ++ [ sandboxPatch ];
  postInstall = (old.postInstall or "") + ''
    install -Dm444 ${policy} \
      "$out/share/d2b/bazel/seccomp-policy.json"
  '';
  postFixup = (old.postFixup or "") + ''
    install -Dm444 ${policy} \
      "$out/share/d2b/bazel/seccomp-policy.json"
  '';
  passthru = (old.passthru or { }) // {
    d2bSeccomp = {
      inherit policy sandboxPatch;
      policyName = "d2b-bazel-action-seccomp-v1";
      capabilityAbi = "d2b-bazel-seccomp-abi-v1";
      sourceUrl = "https://github.com/bazelbuild/bazel/releases/download/8.6.0/bazel-8.6.0-dist.zip";
      sourceHash = "sha256-W22eB0IzHNZe3xaF8AZOkUTDCic3NXkypdqSDY61Su0=";
      policySha256 = builtins.hashFile "sha256" policy;
      patchSha256 = builtins.hashFile "sha256" sandboxPatch;
      loadPoint = "after-sandbox-construction-before-action-command-exec";
      noNetwork = true;
      noFallback = true;
      ptraceRequests = [
        "PTRACE_TRACEME"
        "PTRACE_SETOPTIONS"
        "PTRACE_CONT"
        "PTRACE_DETACH"
      ];
      futureChildPidMatching = false;
      pidNamespace = "CLONE_NEWPID";
      userspaceCeilingMs = 10000;
      pendingState = "pending-kernel-cleanup";
      releaseRecord = "D2B-BZLEXEC-SANDBOX-CONSUMING-REAP-RELEASE";
      diagnostics = [
        {
          owner = "patched-sandbox";
          stage = "SANDBOX_NAMESPACE";
          code = "D2B-BZLEXEC-SANDBOX-NAMESPACE";
        }
        {
          owner = "patched-sandbox";
          stage = "SANDBOX_PTRACE_POLICY";
          code = "D2B-BZLEXEC-SANDBOX-PTRACE-POLICY";
        }
        {
          owner = "patched-sandbox";
          stage = "SANDBOX_MONITOR";
          code = "D2B-BZLEXEC-SANDBOX-MONITOR";
        }
        {
          owner = "patched-sandbox";
          stage = "SANDBOX_KILL";
          code = "D2B-BZLEXEC-SANDBOX-KILL";
        }
        {
          owner = "patched-sandbox";
          stage = "SANDBOX_REAP";
          code = "D2B-BZLEXEC-SANDBOX-REAP";
        }
        {
          owner = "patched-sandbox";
          stage = "SANDBOX_CEILING";
          code = "D2B-BZLEXEC-SANDBOX-CEILING";
        }
        {
          owner = "patched-sandbox";
          stage = "SANDBOX_PENDING_KERNEL_CLEANUP";
          code = "D2B-BZLEXEC-SANDBOX-PENDING-KERNEL-CLEANUP";
        }
        {
          owner = "patched-sandbox";
          stage = "SANDBOX_CLEANUP";
          code = "D2B-BZLEXEC-SANDBOX-CLEANUP";
        }
      ];
    };
  };
  meta = (old.meta or { }) // {
    platforms = [ "x86_64-linux" "aarch64-linux" ];
    mainProgram = "bazel";
  };
})
