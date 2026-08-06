{ pkgs, system, flakeRoot, ... }:

let
  bazel = import (flakeRoot + "/pkgs/bazel-8.6.0-seccomp") { inherit pkgs; };
  supervisor =
    import (flakeRoot + "/pkgs/d2b-bazel-exec-supervisor") { inherit pkgs; };
  policy = builtins.fromJSON (builtins.readFile
    (flakeRoot + "/pkgs/bazel-8.6.0-seccomp/seccomp-policy.json"));
  flakeText = builtins.readFile (flakeRoot + "/flake.nix");
  bazelText = builtins.readFile
    (flakeRoot + "/pkgs/bazel-8.6.0-seccomp/default.nix");
  patchText = builtins.readFile
    (flakeRoot + "/pkgs/bazel-8.6.0-seccomp/linux-sandbox-seccomp.patch");
  supervisorText = builtins.readFile
    (flakeRoot + "/tests/tools/d2b-bazel-exec-supervisor/supervisor.c");
  toolchain = bazel.passthru.d2bSeccomp;
in
{
  "bazel-toolchain/native-system-is-supported" = {
    expr = builtins.elem system [ "x86_64-linux" "aarch64-linux" ];
    expected = true;
  };

  "bazel-toolchain/version-and-source" = {
    expr = {
      version = bazel.version;
      sourceUrl = toolchain.sourceUrl;
      sourceHash = toolchain.sourceHash;
      mainProgram = bazel.meta.mainProgram;
    };
    expected = {
      version = "8.6.0";
      sourceUrl =
        "https://github.com/bazelbuild/bazel/releases/download/8.6.0/bazel-8.6.0-dist.zip";
      sourceHash =
        "sha256-W22eB0IzHNZe3xaF8AZOkUTDCic3NXkypdqSDY61Su0=";
      mainProgram = "bazel";
    };
  };

  "bazel-toolchain-single-devshell-provider" = {
    expr =
      (builtins.match ".*bazelSeccomp.*" flakeText != null)
      && (builtins.match ".*bazel_8.*" flakeText == null)
      && (builtins.match ".*Bazelisk.*" flakeText == null);
    expected = true;
  };

  "bazel-toolchain-policy-load-boundary" = {
    expr = {
      inherit (toolchain) loadPoint noNetwork noFallback;
      policyName = toolchain.policyName;
      policyFile = builtins.match ".*seccomp-policy\\.json.*" bazelText != null;
      patchFile =
        builtins.match ".*linux-sandbox-seccomp\\.patch.*" bazelText != null;
      sourcePatchLoad =
        builtins.match ".*D2BPrepareActionPolicy\\(\\).*" patchText != null;
      sandboxDiagnosticCodes = map (diagnostic: diagnostic.code)
        toolchain.diagnostics;
    };
    expected = {
      loadPoint = "after-sandbox-construction-before-action-command-exec";
      noNetwork = true;
      noFallback = true;
      policyName = "d2b-bazel-action-seccomp-v1";
      policyFile = true;
      patchFile = true;
      sourcePatchLoad = true;
      sandboxDiagnosticCodes = [
        "D2B-BZLEXEC-SANDBOX-NAMESPACE"
        "D2B-BZLEXEC-SANDBOX-PTRACE-POLICY"
        "D2B-BZLEXEC-SANDBOX-MONITOR"
        "D2B-BZLEXEC-SANDBOX-KILL"
        "D2B-BZLEXEC-SANDBOX-REAP"
        "D2B-BZLEXEC-SANDBOX-CEILING"
        "D2B-BZLEXEC-SANDBOX-PENDING-KERNEL-CLEANUP"
        "D2B-BZLEXEC-SANDBOX-CLEANUP"
      ];
    };
  };

  "bazel-toolchain-ptrace-policy-shape" = {
    expr = {
      requests = map (request: request.name) policy.ptrace.requests;
      futureChildPidMatching = policy.ptrace.futureChildPidMatching;
      data = map (request: request.data.value) policy.ptrace.requests;
      pointers = map (request: {
        address = request.address.type;
        data = request.data.type;
      }) policy.ptrace.requests;
      deniedSocket = builtins.elem "socket" policy.deniedSyscalls;
      deniedRing = builtins.elem "io_uring_setup" policy.deniedSyscalls;
      deniedPidfd = builtins.elem "pidfd_getfd" policy.deniedSyscalls;
    };
    expected = {
      requests = [
        "PTRACE_TRACEME"
        "PTRACE_SETOPTIONS"
        "PTRACE_CONT"
        "PTRACE_DETACH"
      ];
      futureChildPidMatching = false;
      data = [ 0 16 0 0 ];
      pointers = [
        { address = "void *"; data = "void *"; }
        { address = "void *"; data = "void *"; }
        { address = "void *"; data = "void *"; }
        { address = "void *"; data = "void *"; }
      ];
      deniedSocket = true;
      deniedRing = true;
      deniedPidfd = true;
    };
  };

  "bazel-toolchain-supervisor-contract" = {
    expr = {
      protocolVersion = supervisor.passthru.protocolVersion;
      privateExecutableFd =
        supervisor.passthru.protocol.privateExecutableFd;
      statusFd = supervisor.passthru.protocol.statusFd;
      retainedBufferBytes =
        supervisor.passthru.protocol.status.retainedBufferBytes;
      noStatusOverlongProbe =
        supervisor.passthru.protocol.status.noStatusOverlongProbe;
      linuxMinimum = supervisor.passthru.linuxMinimum;
      capSysPtrace = supervisor.passthru.yama.capSysPtrace;
      exactCalls = map (call: call.request)
        supervisor.passthru.protocol.ptraceCalls;
      sourceHasCalls =
        builtins.match ".*PTRACE_DETACH.*" supervisorText != null;
    };
    expected = {
      protocolVersion = 1;
      privateExecutableFd = 9;
      statusFd = 8;
      retainedBufferBytes = 27;
      noStatusOverlongProbe = true;
      linuxMinimum = "3.19";
      capSysPtrace = false;
      exactCalls = [
        "PTRACE_TRACEME"
        "PTRACE_SETOPTIONS"
        "PTRACE_CONT"
        "PTRACE_DETACH"
      ];
      sourceHasCalls = true;
    };
  };
}
