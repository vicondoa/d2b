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
  flakeRoot,
}:

let
  flake = builtins.getFlake (toString flakeRoot);
  nixpkgs = flake.inputs.nixpkgs;
  defaultSystem = "x86_64-linux";

  pkgsFor =
    system:
    import nixpkgs {
      inherit system;
      config = {
        allowUnsupportedSystem = true;
      };
    };

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
      nixling.site = {
        waylandUser = "alice";
        launcherUsers = [ "alice" ];
        yubikey.enable = false;
      };
      nixling.envs.work = {
        lanSubnet = "10.20.0.0/24";
        uplinkSubnet = "192.0.2.0/30";
      };
      nixling.vms.corp-vm = {
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

      nixos = nixpkgs.lib.nixosSystem {
        inherit system;
        pkgs = pkgsFor system;
        modules = [
          flake.nixosModules.default
          baseModule
          override
        ];
      };

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
  inherit evalCase mkBatch baseModule defaultSystem pkgsFor;
}
