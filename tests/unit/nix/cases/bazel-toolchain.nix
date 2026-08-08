{ pkgs, system, flakeRoot, ... }:

let
  bazel = import (flakeRoot + "/pkgs/bazel-8.6.0-seccomp") { inherit pkgs; };
  supervisor =
    import (flakeRoot + "/pkgs/d2b-bazel-exec-supervisor") { inherit pkgs; };
  policy = builtins.fromJSON (builtins.readFile
    (flakeRoot + "/pkgs/bazel-8.6.0-seccomp/seccomp-policy.json"));
  flakeText = builtins.readFile (flakeRoot + "/flake.nix");
  bazelrcText = builtins.readFile (flakeRoot + "/.bazelrc");
  sandboxRuleText = builtins.readFile
    (flakeRoot + "/bazel/rules/sandboxed_action.bzl");
  bazelText = builtins.readFile
    (flakeRoot + "/pkgs/bazel-8.6.0-seccomp/default.nix");
  patchText = builtins.readFile
    (flakeRoot + "/pkgs/bazel-8.6.0-seccomp/linux-sandbox-seccomp.patch");
  supervisorText = builtins.readFile
    (flakeRoot + "/tests/tools/d2b-bazel-exec-supervisor/supervisor.c");
  plantText = builtins.readFile
    (flakeRoot + "/tests/tools/d2b-bazel-exec-supervisor/sandbox-crash-plant.c");
  golden = builtins.fromJSON (builtins.readFile
    (flakeRoot + "/tests/golden/bazel-toolchain.json"));
  supervisorGolden = builtins.fromJSON (builtins.readFile
    (flakeRoot + "/tests/golden/bazel-exec-supervisor.json"));
  currentSourceHashes = {
    policy = builtins.hashFile "sha256"
      (flakeRoot + "/pkgs/bazel-8.6.0-seccomp/seccomp-policy.json");
    patch = builtins.hashFile "sha256"
      (flakeRoot + "/pkgs/bazel-8.6.0-seccomp/linux-sandbox-seccomp.patch");
    supervisor = builtins.hashFile "sha256"
      (flakeRoot + "/tests/tools/d2b-bazel-exec-supervisor/supervisor.c");
    plant = builtins.hashFile "sha256"
      (flakeRoot + "/tests/tools/d2b-bazel-exec-supervisor/sandbox-crash-plant.c");
    expression = builtins.hashFile "sha256"
      (flakeRoot + "/pkgs/d2b-bazel-exec-supervisor/default.nix");
  };
  zeroSha256 = builtins.concatStringsSep "" (builtins.genList (_: "0") 64);
  supportedSystems = [ "x86_64-linux" "aarch64-linux" ];
  nativeDigestsAreNonzero = record:
    builtins.all (nativeSystem:
      let output = record.nativeOutputs.${nativeSystem};
      in builtins.all (digest: digest != zeroSha256) [
        output.derivationSha256
        output.narSha256
        output.executableSha256
      ]) supportedSystems;
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
      (pkgs.lib.hasInfix "bazelSeccomp" flakeText)
      && !(pkgs.lib.hasInfix "bazel_8" flakeText)
      && !(pkgs.lib.hasInfix "Bazelisk" flakeText);
    expected = true;
  };

  "bazel-toolchain-policy-load-boundary" = {
    expr = {
      inherit (toolchain) loadPoint noNetwork noFallback derivationSha256Method;
      policyName = toolchain.policyName;
      policyFile = pkgs.lib.hasInfix "seccomp-policy.json" bazelText;
      patchFile = pkgs.lib.hasInfix "linux-sandbox-seccomp.patch" bazelText;
      policyEnv = pkgs.lib.hasInfix "D2B_BAZEL_SECCOMP_POLICY" bazelText;
      sourcePatchLoad = pkgs.lib.hasInfix "D2BPrepareActionPolicy()" patchText;
      goldenDerivationSha256Method = golden.derivationSha256Method;
      goldenNativeDigestsAreNonzero = nativeDigestsAreNonzero golden;
      supervisorNativeDigestsAreNonzero =
        nativeDigestsAreNonzero supervisorGolden;
      nativeOutputs = map (nativeSystem:
        let output = golden.nativeOutputs.${nativeSystem};
        in {
          derivationSha256 = output.derivationSha256;
          narSha256 = output.narSha256;
          executableSha256 = output.executableSha256;
          filterLoad = output.startupProbe.filterLoad;
        }) supportedSystems;
      sandboxDiagnosticCodes = map (diagnostic: diagnostic.code)
        toolchain.diagnostics;
    };
    expected = {
      loadPoint = "after-sandbox-construction-before-action-command-exec";
      noNetwork = true;
      noFallback = true;
      derivationSha256Method = "raw-drv-file-sha256";
      policyName = "d2b-bazel-action-seccomp-v1";
      policyFile = true;
      patchFile = true;
      policyEnv = true;
      sourcePatchLoad = true;
      goldenDerivationSha256Method = "raw-drv-file-sha256";
      goldenNativeDigestsAreNonzero = true;
      supervisorNativeDigestsAreNonzero = true;
      nativeOutputs = [
        {
          derivationSha256 =
            "3bd25f12e8446d9391ea27c302023b538858d915c02d5e7f9010963bccfd3490";
          narSha256 =
            "197e2e792a7a3cf72bc9a5892b418d4abcce590dad969d23619f2bb492486be5";
          executableSha256 =
            "7cbf33369f34c39ceaed716ab26f4c37d32df009f290243e301e0cf8b83eafa8";
          filterLoad = "observed";
        }
        {
          derivationSha256 =
            "71844ea14ac76e4135e2fd8f49165517caaae05be0899aacf20360805cb5450e";
          narSha256 =
            "8318412b0722765167051e15ff735819b8e9f0b2ab619e1653975c25db3bbb16";
          executableSha256 =
            "9898ce560dc199283b26c9f0efee8a217c53f45d1687a0c6b0c36cb9a2d7ee59";
          filterLoad = "observed";
        }
      ];
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
      x32RejectedBeforeDispatch =
        policy.x86X32SyscallBit.rejectedBeforeDispatch;
      x32Denial = policy.x86X32SyscallBit.denial;
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
      x32RejectedBeforeDispatch = true;
      x32Denial = {
        action = "errno";
        errno = "EACCES";
        value = 13;
      };
    };
  };

  "bazel-toolchain-strategy-and-plants" = {
    expr = {
      strategy = pkgs.lib.hasInfix "common --spawn_strategy=sandboxed" bazelrcText;
      rustcStrategy = pkgs.lib.hasInfix "common --strategy=Rustc=sandboxed" bazelrcText;
      testStrategy = pkgs.lib.hasInfix "common --strategy=TestRunner=sandboxed" bazelrcText;
      effectiveObservation =
        pkgs.lib.hasInfix "d2b_validate_effective_strategies" sandboxRuleText;
      strategyLock = pkgs.lib.hasInfix "D2B_BAZEL_STRATEGY_LOCK" bazelText;
      noStrategyOverride =
        pkgs.lib.hasInfix "strategyOverrides = false" bazelText;
      x32Guard = pkgs.lib.hasInfix "BPF_JMP | BPF_JSET | BPF_K" patchText;
      livenessPath = pkgs.lib.hasInfix "--liveness-path" plantText;
      barrierPath = pkgs.lib.hasInfix "--barrier-path" plantText;
      beyondCeilingStage =
        pkgs.lib.hasInfix "case PLANT_BEYOND_CEILING:" plantText;
      plantIgnoresTerm =
        pkgs.lib.hasInfix "signal(SIGTERM, SIG_IGN)" plantText;
    };
    expected = {
      strategy = true;
      rustcStrategy = true;
      testStrategy = true;
      effectiveObservation = true;
      strategyLock = true;
      noStrategyOverride = true;
      x32Guard = true;
      livenessPath = true;
      barrierPath = true;
      beyondCeilingStage = true;
      plantIgnoresTerm = true;
    };
  };

  "bazel-toolchain-supervisor-contract" = {
    expr = {
      derivationSha256Method =
        supervisor.passthru.derivationSha256Method;
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
      sourceHasCalls = pkgs.lib.hasInfix "PTRACE_DETACH" supervisorText;
    };
    expected = {
      derivationSha256Method = "raw-drv-file-sha256";
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

  "bazel-toolchain-current-package-hashes" = {
    expr = builtins.all
      (hash: builtins.isString hash
        && builtins.match "[0-9a-fA-F]{64}" hash != null)
      (builtins.attrValues currentSourceHashes)
      && pkgs.lib.hasInfix "bazelSourceIdentityGate" flakeText;
    expected = true;
  };

  "bazel-toolchain-native-check-surface" = {
    expr = builtins.all
      (check: pkgs.lib.hasInfix check flakeText)
      [
        "broker-production-dependency-policy"
        "guest-shell-runner-static-dependency-policy"
        "broker-production-package-policy"
        "guest-real-libshpool-package-policy"
        "broker-host-artifact-contract"
        "guest-static-elf"
      ]
      && flakeText != "";
    expected = true;
  };
}
