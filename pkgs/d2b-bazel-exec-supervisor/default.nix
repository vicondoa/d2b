{ pkgs }:

let
  staticPkgs = pkgs.pkgsStatic;
  readelf = "${staticPkgs.binutils.bintools}/bin/readelf";
in
staticPkgs.stdenv.mkDerivation {
  pname = "d2b-bazel-exec-supervisor";
  version = "0.0.0-spec003";
  src = ../../tests/tools/d2b-bazel-exec-supervisor;
  dontUnpack = true;
  dontConfigure = true;
  strictDeps = true;
  nativeBuildInputs = [ staticPkgs.binutils ];
  buildPhase = ''
    runHook preBuild
    "$CC" \
      -std=c11 \
      -O2 \
      -Wall \
      -Wextra \
      -Werror \
      -Wno-unused-parameter \
      -fno-pie \
      -no-pie \
      -static \
      "$src/supervisor.c" \
      -o d2b-bazel-exec-supervisor
    runHook postBuild
  '';
  doCheck = true;
  checkPhase = ''
    runHook preCheck
    ${readelf} -h d2b-bazel-exec-supervisor >/dev/null
    ${readelf} -l d2b-bazel-exec-supervisor > supervisor.program-headers
    ! grep -Fq 'Requesting program interpreter' supervisor.program-headers
    if ${readelf} -d d2b-bazel-exec-supervisor > supervisor.dynamic 2> supervisor.dynamic.err; then
      ! grep -Fq '(NEEDED)' supervisor.dynamic
    else
      grep -qi 'no dynamic section' supervisor.dynamic.err
    fi
    runHook postCheck
  '';
  installPhase = ''
    runHook preInstall
    install -Dm755 d2b-bazel-exec-supervisor \
      "$out/bin/d2b-bazel-exec-supervisor"
    runHook postInstall
  '';
  passthru = {
    derivationSha256Method = "raw-drv-file-sha256";
    protocolVersion = 1;
    protocol = {
      privateExecutableFd = 9;
      statusFd = 8;
      execError = {
        magic = "D2BE";
        version = 1;
        recordBytes = 8;
        overlongRule = "one-byte-probe-after-one-record";
      };
      status = {
        magic = "D2BS";
        version = 1;
        headerBytes = 8;
        retainedBufferBytes = 27;
        frames = {
          READY = { type = 1; payloadBytes = 0; };
          EXECUTED = { type = 2; payloadBytes = 0; };
          EXITED = { type = 3; payloadBytes = 1; };
          SIGNALED = { type = 4; payloadBytes = 1; };
        };
        order = [ "READY" "EXECUTED" "terminal" "EOF" ];
        noStatusOverlongProbe = true;
      };
      ptraceCalls = [
        {
          request = "PTRACE_TRACEME";
          pid = "0";
          address = "(void *)0";
          data = "(void *)0";
        }
        {
          request = "PTRACE_SETOPTIONS";
          pid = "child";
          address = "(void *)0";
          data = "(void *)(uintptr_t)PTRACE_O_TRACEEXEC";
        }
        {
          request = "PTRACE_CONT";
          pid = "child";
          address = "(void *)0";
          data = "(void *)0";
        }
        {
          request = "PTRACE_DETACH";
          pid = "child";
          address = "(void *)0";
          data = "(void *)0";
        }
      ];
    };
    linuxMinimum = "3.19";
    yama = {
      assumption = "parent-child";
      acceptedUnprivilegedModes = [ 0 1 ];
      refusedModes = [ 2 3 ];
      capSysPtrace = false;
    };
    preHelperPredicates = [
      {
        owner = "toolchain-startup";
        stage = "TOOLCHAIN_PTRACE_KERNEL";
        code = "D2B-BZLEXEC-TOOLCHAIN-PTRACE-KERNEL";
        causingInput = "linux-minimum-3.19";
      }
      {
        owner = "toolchain-startup";
        stage = "TOOLCHAIN_PTRACE_YAMA";
        code = "D2B-BZLEXEC-TOOLCHAIN-PTRACE-YAMA";
        causingInput = "yama-parent-child-mode";
      }
      {
        owner = "toolchain-startup";
        stage = "TOOLCHAIN_PTRACE_PROBE";
        code = "D2B-BZLEXEC-TOOLCHAIN-PTRACE-PROBE";
        causingInput = "immutable-ptrace-startup-probe";
      }
    ];
    sourceFiles = [
      "tests/tools/d2b-bazel-exec-supervisor/supervisor.c"
      "tests/tools/d2b-bazel-exec-supervisor/sandbox-crash-plant.c"
    ];
  };
  meta = {
    description = "Static immutable Bazel action execution supervisor";
    mainProgram = "d2b-bazel-exec-supervisor";
    platforms = [ "x86_64-linux" "aarch64-linux" ];
  };
}
