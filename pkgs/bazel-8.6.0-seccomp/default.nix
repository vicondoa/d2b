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
    wrapProgram "$out/bin/bazel" \
      --set D2B_BAZEL_SECCOMP_POLICY \
      "$out/share/d2b/bazel/seccomp-policy.json" \
      --set D2B_BAZEL_STRATEGY_LOCK d2b-bazel-sandbox-v1 \
      --unset BAZELRC \
      --unset BAZELISK_HOME \
      --unset BAZELISK_USER_AGENT \
      --unset D2B_BAZEL_LINUX_SANDBOX \
      --run '
        d2b_workspace_rc="$(pwd -P 2>/dev/null)/.bazelrc"
        if [ ! -f "$d2b_workspace_rc" ]; then
          printf "%s\n" D2B-BZLNET-RC
          printf "%s\n" "correction=Run the pinned Bazel from a repository with its governed .bazelrc."
          printf "%s\n" "retry=make test-flake"
          exit 64
        fi
        for d2b_arg in "$@"; do
          case "$d2b_arg" in
            --spawn_strategy=*|--spawn_strategy|--strategy=*|--strategy|--strategy_regexp=*|--strategy_regexp|--test_strategy=*|--test_strategy|--genrule_strategy=*|--genrule_strategy|--worker_strategy=*|--worker_strategy|--remote_executor=*|--remote_executor|--remote_cache=*|--remote_cache|--experimental_spawn_scheduler|--experimental_remote_downloader=*|--experimental_remote_downloader|--bazelrc=*|--bazelrc|--system_rc|--home_rc|--workspace_rc|--noworkspace_rc|--ignore_all_rc_files|--config=*|--config|--action_env=*|--action_env|--host_action_env=*|--host_action_env|--test_env=*|--test_env|--client_env=*)
              printf "%s\n" D2B-BZLNET-STRATEGY
              printf "%s\n" "correction=Use the repository .bazelrc and the pinned sandboxed strategy without rc, config, environment, or strategy overrides."
              printf "%s\n" "retry=make test-flake"
              exit 64
              ;;
          esac
        done
        set -- --nosystem_rc --nohome_rc --noworkspace_rc "--bazelrc=$d2b_workspace_rc" "$@"
      '
  '';
  passthru = (old.passthru or { }) // {
    d2bSeccomp = {
      inherit policy sandboxPatch;
      derivationSha256Method = "raw-drv-file-sha256";
      policyName = "d2b-bazel-action-seccomp-v1";
      capabilityAbi = "d2b-bazel-seccomp-abi-v1";
      sourceUrl = "https://github.com/bazelbuild/bazel/releases/download/8.6.0/bazel-8.6.0-dist.zip";
      sourceHash = "sha256-W22eB0IzHNZe3xaF8AZOkUTDCic3NXkypdqSDY61Su0=";
      policySha256 = builtins.hashFile "sha256" policy;
      patchSha256 = builtins.hashFile "sha256" sandboxPatch;
      loadPoint = "after-sandbox-construction-before-action-command-exec";
      noNetwork = true;
      noFallback = true;
      strategy = "sandboxed";
      strategyLock = "d2b-bazel-sandbox-v1";
      strategyOverrides = false;
      rcPolicy = "workspace-only";
      actionEnvPolicy = "strict-pinned";
      testEnvPolicy = "strict-empty";
      retryCommand = "make test-flake";
      x86X32SyscallBit = {
        rejectedBeforeDispatch = true;
        denialErrno = "EACCES";
        denialValue = 13;
      };
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
