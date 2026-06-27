# W3a-1 (test-1): shared evaluator for the consolidated assertions-eval
# and observability-eval harnesses.
#
# Each consumer (`assertions.nix`, `observability.nix`) calls
# `mkBatch { cases = { name = { override, expectedSubstring, ... }; }; }`
# and gets back a single attrset shaped like:
#
#   { <case-name> = {
#       expectedSubstring = "...";        # copied from input for the wrapper
#       evalSucceeded     = <bool>;       # tryEval (deepSeq config.assertions)
#       throwMessage      = "...";        # populated when evalSucceeded == false
#       failingMessages   = [ "..." ];    # config.assertions where assertion == false
#       allMessages       = [ "..." ];    # ALL assertion messages (debug aid)
#       warnings          = [ "..." ];    # config.warnings
#     }; ... }
#
# The shell wrapper then runs ONE `nix-instantiate --eval --strict --json`
# against this attrset and per-case asserts:
#   - eval-failure cases: throwMessage contains expectedSubstring
#                         OR failingMessages contains expectedSubstring
#   - success cases:      failingMessages == [ ] AND warnings predicate
#                         (defined per-case in JSON-shape via extra fields)
#
# Replaces the old per-case `nix-instantiate --eval` invocation (31 cases
# in assertions-eval, 23 in observability-eval) with one batched eval.
{
  # Two ways to supply nixpkgs + the d2b module set:
  #   * flakeRoot — re-`getFlake`s the repo (the bash gates' path; flakeRoot
  #     is a real working-tree path).
  #   * nixpkgs + d2bModule — direct injection, used by the in-flake
  #     nix-unit check where `flakeRoot = ./.` would resolve to a non-git
  #     store path. Provide exactly one of the two.
  flakeRoot ? null,
  nixpkgs ? null,
  d2bModule ? null,
}:

let
  flake =
    if flakeRoot != null
    then builtins.getFlake "git+file://${toString flakeRoot}"
    else null;
  resolvedNixpkgs =
    if nixpkgs != null then nixpkgs
    else if flake != null then flake.inputs.nixpkgs
    else throw "shared.nix: provide either flakeRoot or nixpkgs";
  resolvedD2bModule =
    if d2bModule != null then d2bModule
    else if flake != null then flake.nixosModules.default
    else throw "shared.nix: provide either flakeRoot or d2bModule";
  lib = resolvedNixpkgs.lib;
  defaultSystem = "x86_64-linux";

  # ---------------------------------------------------------------------
  # Minimal NixOS option surface (the eval-speed win).
  #
  # The assertions gate only ever forces `config.assertions`, which reads
  # just `config.users.users` (membership) plus `config.d2b.*`. It
  # does NOT need nixpkgs' ~1,370-module `nixosSystem` baseModules — those
  # only matter for building a real system. Booting them per case cost
  # ~28s each (26 cases ≈ 13 min).
  #
  # Instead we `lib.evalModules` with ONLY:
  #   * nixpkgs' self-contained `misc/assertions.nix` (declares the
  #     `assertions` / `warnings` options d2b writes to), and
  #   * sink declarations (`types.anything`) for every other top-level
  #     NixOS namespace d2b's modules *write* to, so those definitions
  #     are accepted. The sinks are never forced (config.assertions does
  #     not read them), so they cost nothing.
  # Marginal per-case cost drops from ~28s to ~0.6s. This mirrors how
  # nixpkgs tests its own module system (lib/tests/modules.sh): minimal
  # `evalModules` over fixtures, never `nixosSystem`.
  assertionsModule = resolvedNixpkgs + "/nixos/modules/misc/assertions.nix";

  # Top-level NixOS namespaces d2b's modules assign to. If d2b
  # grows a write to a new top-level NixOS namespace, add it here (a
  # missing sink surfaces loudly as `option <ns> does not exist`, never
  # as a silent wrong result).
  sinkNamespaces = [
    "users"
    "system"
    "services"
    "environment"
    "boot"
    "networking"
    "security"
    "documentation"
    "time"
    "nix"
    "i18n"
    "hardware"
    "fileSystems"
    "swapDevices"
    "powerManagement"
    "programs"
    "console"
    "fonts"
    "sound"
    "virtualisation"
    "specialisation"
    "zramSwap"
    "xdg"
    "qt"
  ];

  mkSink =
    name:
    { lib, ... }:
    {
      options.${name} = lib.mkOption {
        type = lib.types.anything;
        default = { };
      };
    };

  # `systemd` needs a faithful-enough shape because the gate reads back
  # two sub-options: `systemd.services` (membership checks) and
  # `systemd.tmpfiles.rules` (a `listOf str` that MUST concatenate across
  # modules — a bare `types.anything` reports conflicting list defs
  # instead of merging). Everything else under `systemd` stays freeform.
  systemdSink =
    { lib, ... }:
    {
      options.systemd = lib.mkOption {
        default = { };
        type = lib.types.submodule {
          freeformType = lib.types.attrsOf lib.types.anything;
          options.services = lib.mkOption {
            type = lib.types.attrsOf lib.types.anything;
            default = { };
          };
          options.tmpfiles = lib.mkOption {
            default = { };
            type = lib.types.submodule {
              freeformType = lib.types.attrsOf lib.types.anything;
              options.rules = lib.mkOption {
                type = lib.types.listOf lib.types.str;
                default = [ ];
              };
            };
          };
        };
      };
    };

  # `nixpkgs` is read by vm-evaluator.nix (`config.nixpkgs.config` and
  # `config.nixpkgs.overlays`), so its sink seeds both attrs.
  nixpkgsSink =
    { lib, ... }:
    {
      options.nixpkgs = lib.mkOption {
        type = lib.types.anything;
        default = {
          config = { };
          overlays = [ ];
        };
      };
    };

  sinkModules = (builtins.map mkSink sinkNamespaces) ++ [
    systemdSink
    nixpkgsSink
  ];

  # Memoize pkgs per system at the top level so the (pure, deterministic)
  # `import nixpkgs { inherit system; ... }` thunk is shared across every
  # case with the same system instead of being re-imported per case.
  importPkgs =
    system:
    import resolvedNixpkgs {
      inherit system;
      config = {
        allowUnsupportedSystem = true;
      };
    };
  pkgsX86 = importPkgs "x86_64-linux";
  pkgsAarch64 = importPkgs "aarch64-linux";
  pkgsFor =
    system:
    if system == "aarch64-linux" then pkgsAarch64 else if system == "x86_64-linux" then pkgsX86 else importPkgs system;

  # Base consumer module identical to the one mk_expr produced in the
  # legacy bash harness. Every case stacks its override on top.
  baseModule = (
    { lib, ... }:
    {
      boot.loader.grub.enable = false;
      boot.loader.systemd-boot.enable = false;
      fileSystems."/" = {
        device = "tmpfs";
        fsType = "tmpfs";
      };
      environment.etc."machine-id".text = "00000000000000000000000000000000";
      system.stateVersion = "25.11";
      users.users.alice = {
        isNormalUser = true;
        uid = 1000;
      };
      d2b.site = {
        waylandUser = "alice";
        launcherUsers = [ "alice" ];
        yubikey.enable = false;
      };
      d2b.envs.work = {
        lanSubnet = "10.20.0.0/24";
        uplinkSubnet = "192.0.2.0/30";
      };
      d2b.vms.corp-vm = {
        enable = true;
        env = "work";
        index = 10;
        ssh.user = "alice";
        config = {
          networking.hostName = lib.mkDefault "corp-vm";
          users.users.alice = {
            isNormalUser = true;
            uid = 1000;
          };
        };
      };
    }
  );

  # Build the raw module-system evaluation for ONE case. Shared by the
  # batched `evalCase` below and by the shell gate's per-case fallback
  # (`mk_expr` in tests/assertions-eval.sh) so both paths use identical
  # eval semantics.
  mkEval =
    { override, system ? defaultSystem }:
    lib.evalModules {
      modules = [
        assertionsModule
        resolvedD2bModule
        baseModule
        override
      ]
      ++ sinkModules;
      specialArgs = {
        inherit lib;
        pkgs = pkgsFor system;
        modulesPath = resolvedNixpkgs + "/nixos/modules";
      };
    };

  # Evaluate ONE case. Force `config.assertions` via tryEval so any
  # mkOption type-check throw or `assert lib.assertMsg ... ` somewhere
  # in the module body surfaces as `evalSucceeded = false` with the
  # captured throw payload extracted on a best-effort basis.
  #
  # NixOS treats `assertions = [{ assertion = bool, message = string }]`
  # as data, so reading the list itself does NOT throw — that lets us
  # accumulate failing messages without each failed-assertion record
  # also bubbling up.
  evalCase = caseSpec:
    let
      system = caseSpec.system or defaultSystem;
      override = caseSpec.override;

      nixos = mkEval { inherit override system; };

      # Try to read the assertions list. If a module-evaluation throw
      # fires before assertions are computable, tryEval catches it.
      #
      # IMPORTANT (W4a R1 rust-1 + R2 rust-1): we must NOT deep-force
      # the full `xs` list — that would force `.message` on every
      # record, including passing assertions whose message thunks
      # reference values only safe to read when the assertion is
      # false (see `nixos/modules/tasks/filesystems.nix` line 452
      # forcing `fileSystems'.cycle` inside its message string).
      # Forcing those throws would make unrelated cases falsely
      # report `evalSucceeded = false`.
      #
      # Strategy: deepSeq a projection that keeps just the
      # `.assertion` booleans, leaving `.message` thunks untouched.
      # Use `deepSeq forces result` (the two-arg form) rather than
      # binding to a `let _ = ...; in ...` pattern — Nix is lazy,
      # so an unused `let` binding never forces evaluation. The
      # two-arg form forces `forces` before returning `result`.
      assertionsAttempt = builtins.tryEval (
        let
          xs = nixos.config.assertions;
          assertionBools = builtins.map (a: a.assertion) xs;
        in
        builtins.deepSeq assertionBools xs
      );

      warningsAttempt = builtins.tryEval (
        let
          ws = nixos.config.warnings;
          _ = builtins.deepSeq ws null;
        in
        ws
      );

      asserts = if assertionsAttempt.success then assertionsAttempt.value else [ ];
      # Only force `.message` on failing assertions; many NixOS modules
      # build the message thunk from values that are only safe to read
      # when the assertion is false (cf. tasks/filesystems.nix forcing
      # `fileSystems'.cycle` inside the message string), so forcing
      # `.message` on passing assertions can throw.
      failingMessages = builtins.map (a: a.message) (
        builtins.filter (a: !a.assertion) asserts
      );
    in
    {
      inherit (caseSpec) expectedSubstring;
      kind = caseSpec.kind or "expect-failure"; # or "expect-success"
      evalSucceeded = assertionsAttempt.success;

      # `tryEval` swallows the throw message itself. Wrapper falls back
      # to a per-case focused `nix-instantiate --eval` ONLY for cases
      # where evalSucceeded == false AND failingMessages does not
      # contain expectedSubstring (i.e. genuine throw cases that
      # neither bath emitted nor recorded via assertions). The shared
      # batch eval still wins on the 90%+ assertion-list cases.
      throwMessage = "";

      inherit failingMessages;
      # builtins.length on the full list is safe (no .message forcing)
      # and lets the wrapper sanity-check that we saw a non-empty
      # assertion universe for each case.
      assertionsTotal = builtins.length asserts;
      warnings = if warningsAttempt.success then warningsAttempt.value else [ ];
    };

  mkBatch = { cases }: builtins.mapAttrs (_: spec: evalCase spec) cases;

in
{
  inherit evalCase mkEval mkBatch baseModule defaultSystem pkgsFor lib;
}
