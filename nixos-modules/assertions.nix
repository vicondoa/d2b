# Eval-time validation of the nixling option schema.
#
# All assertions here fire BEFORE any build kicks off, so a typo in
# `nixling.vms.<name>` or an unsupported env name turns into a clear
# eval error instead of a confusing systemd-escape failure or a
# 16-char-truncated `ip link` name at runtime.
#
# The matching env-level assertions (env exists, env+index uniqueness,
# staticIp / env mutual exclusion, env name ≤ 8 chars) live in
# network.nix where the env iteration happens. This file owns the
# per-VM-name and per-env-name format / reserved-prefix checks that
# don't depend on network.nix's iteration of cfg.envs.
{ config, lib, options, ... }:

let
  cfg = config.nixling;
  obsCfg = cfg.observability;
  obsVsockCid = 1000;
  nl = import ./lib.nix { inherit lib; };
  inherit (nl) parseCidr subnetIp;

  pow2 = n:
    lib.foldl' (acc: _: acc * 2) 1 (lib.genList (i: i) n);

  cidrContains = cidr: ip:
    let
      parsed = parseCidr cidr;
      divisor = pow2 (32 - parsed.prefix);
      ipInt = (parseCidr ip).netInt;
    in
    parsed.netInt / divisor == ipInt / divisor;

  # Allowed SSH public-key types. We match by prefix on the key line
  # ("ssh-ed25519 AAAA..."). Adding/removing types here is a deliberate
  # choice — be conservative: legacy RSA<2048, DSA, etc. are NOT in
  # this list.
  sshKeyPrefixes = [
    "ssh-ed25519 "
    "ssh-rsa "
    "ecdsa-sha2-nistp256 "
    "ecdsa-sha2-nistp384 "
    "ecdsa-sha2-nistp521 "
    "sk-ssh-ed25519@openssh.com "
    "sk-ecdsa-sha2-nistp256@openssh.com "
  ];

  hasAllowedSshPrefix = s:
    lib.any (p: lib.hasPrefix p s) sshKeyPrefixes;

  # Eval-time validation of an authorized-keys entry. The entry is
  # either a path (a .pub file on disk, e.g. /etc/nixos/keys/foo.pub)
  # or a string (the literal pubkey content). Either way the first
  # non-empty / non-comment line must start with an allowed prefix
  # AND must not look like a PRIVATE key (the universal disaster
  # signal: "-----BEGIN ... PRIVATE KEY-----").
  authorizedKeyEntryOk = entry:
    let
      raw =
        if builtins.isPath entry || lib.isStorePath entry
        then builtins.readFile entry
        else if builtins.isString entry
        then entry
        else throw "nixling: authorized-key entry must be a path or string";
      lines = lib.splitString "\n" raw;
      # Strip pure-comment and pure-whitespace lines.
      firstReal = lib.findFirst
        (l: let s = lib.removePrefix " " l;
            in s != "" && !(lib.hasPrefix "#" s))
        ""
        lines;
      noPrivateMarker = !(lib.hasInfix "PRIVATE KEY" raw);
    in
    noPrivateMarker && hasAllowedSshPrefix firstReal;

  # Pretty origin for the error message — for a path, name it; for a
  # string, truncate.
  authorizedKeyEntryName = entry:
    if builtins.isPath entry || lib.isStorePath entry
    then toString entry
    else "<literal key string '${lib.substring 0 40 (toString entry)}…'>";

  validateAuthorizedKeys = label: list:
    lib.flatten (lib.imap0
      (i: entry: lib.optional (!authorizedKeyEntryOk entry) {
        assertion = false;
        message = ''
          ${label} entry #${toString i} (${authorizedKeyEntryName entry})
          does not look like a valid SSH public key.

          Allowed types: ed25519, RSA, ECDSA (nistp256/384/521),
          security-key variants.

          Common causes:
          - You handed it a PRIVATE key file (a "-----BEGIN ... PRIVATE
            KEY-----" header was found in the content). Use the
            matching .pub file instead.
          - The file is empty or only contains comments.
          - The key uses an unsupported type (legacy RSA<2048, DSA, ...).
        '';
      })
      list);

  # Auto-declared system VMs (added to cfg.vms by network.nix and, when
  # enabled, by observability-vm.nix) must NOT trip the `sys-`
  # reserved-prefix rule. Derive the allowed set from the auto-net VMs
  # plus the reserved observability stack VM name.
  autoSysVmNames =
    (lib.mapAttrsToList
      (envName: env: env.netName or "sys-${envName}-net")
      cfg.envs)
    ++ lib.optional obsCfg.enable obsCfg.vmName;

  # Systemd-escape identity regex (lower-case alnum and `-`, must
  # start with a LETTER). `^[a-z][a-z0-9-]*$` deliberately excludes:
  #   * `.` (dots — systemd-escape would turn them into `\x2e`)
  #   * `_` (underscores — same)
  #   * `@` (would collide with template-instance separator)
  #   * `/` (path separator)
  #   * uppercase (NixOS option names are case-sensitive but
  #     downstream tooling like `systemctl --type=service` is not
  #     consistent; lower-case avoids the foot-gun)
  #   * leading `-` (looks like a flag)
  #   * leading digit (W2-followup H1: a numeric-prefixed VM/env name
  #     like `42web` produces unit names such as `nixling@42web.service`
  #     and tap names like `42web-l10` which are technically legal but
  #     trip up tooling that treats the leading digit as a numeric
  #     argument — e.g. `ip link show 42web-l10` resolves to the
  #     interface at index 42 first. Requiring a leading letter
  #     matches systemd-escape best practices and avoids the
  #     ambiguity. Stricter than the W2 plan's original
  #     `^[a-z0-9][a-z0-9-]*$`; accepted at panel review.)
  vmNameOk = name:
    builtins.match "^[a-z][a-z0-9-]*$" name != null;

  # Reserved single-name: `launcher` is taken by the polkit-launcher
  # group (`nixling-launcher`) singleton. A VM named `launcher` would
  # produce `nixling-launcher-gpu` etc. users that collide with the
  # group's namespace.
  reservedVmName = name: name == "launcher";

  # Reserved prefix for auto-declared system VMs. User-declared VMs
  # cannot use this prefix because it would shadow / collide with the
  # auto-declared `sys-<env>-net` namespace.
  reservedVmPrefix = name:
    lib.hasPrefix "sys-" name && !(lib.elem name autoSysVmNames);

  # Env name regex (same shape as VM names, no `sys-` prefix
  # restriction — env names like `sys` would still be permitted by
  # this rule, but combined with the IFNAMSIZ-1 ≤ 8 char rule in
  # network.nix the practical surface is small). The check is here
  # rather than in network.nix because it's a pure naming-format
  # rule, not a topology rule.
  #
  # Same leading-letter restriction as vmNameOk (W2-followup H1):
  # env names show up in interface names (`br-<env>-up`, `<env>-l1`)
  # which `ip link` and other tools treat as numeric indices when
  # they start with a digit.
  envNameOk = name:
    builtins.match "^[a-z][a-z0-9-]*$" name != null;

  obsVmDefinitions = lib.filter
    (d: builtins.isAttrs d.value && builtins.hasAttr obsCfg.vmName d.value)
    options.nixling.vms.definitionsWithLocations;

  # Pre-v0.2.0 the framework rejected ANY consumer definition under
  # `nixling.vms.<obsCfg.vmName>` to prevent "user-declared VM collides
  # with auto-declared one" mistakes. In practice that blocked
  # perfectly safe extensions like `ssh.user = "root"` on the obs
  # VM, because the framework's `observability-vm.nix` block already
  # uses `lib.mkDefault` for every value it sets — a consumer extension
  # MERGES on top of it via the module system. The assertion was
  # over-conservative and the check was removed in v0.2.0. We retain
  # `userObsVmDefinitions` purely for diagnostics in other error
  # messages elsewhere.
  userObsVmDefinitions = lib.filter
    (d: !(lib.hasSuffix "/nixos-modules/observability-vm.nix" d.file))
    obsVmDefinitions;

  workloadObsCidPairs = lib.mapAttrsToList
    (name: _vm: {
      inherit name;
      cid = config.nixling.manifest.${name}.observability.vsockCid;
    })
    (lib.filterAttrs
      (name: vm: vm.enable && vm.observability.enable && name != obsCfg.vmName)
      cfg.vms);

  workloadObsCidGroups = lib.groupBy
    (pair: toString pair.cid)
    workloadObsCidPairs;

  collidingWorkloadObsCidGroups = lib.filterAttrs
    (_: pairs: builtins.length pairs > 1)
    workloadObsCidGroups;

  mkCidCollisionPairs = pairs:
    if pairs == [ ] then [ ] else
    let
      first = builtins.head pairs;
      rest = builtins.tail pairs;
    in
    (map (other: {
      vm1 = first.name;
      vm2 = other.name;
      cid = first.cid;
    }) rest)
    ++ mkCidCollisionPairs rest;

  workloadObsCidCollisions =
    lib.flatten (map mkCidCollisionPairs (lib.attrValues collidingWorkloadObsCidGroups));

  reservedObsCidUsers = map (pair: pair.name)
    (lib.filter (pair: pair.cid == obsVsockCid) workloadObsCidPairs);

  vmAssertions = lib.mapAttrsToList
    (name: vm: [
      {
        assertion = vmNameOk name;
        message = "nixling.vms.${name}: VM name must match the "
          + "regex ^[a-z][a-z0-9-]*$ (lowercase alnum + '-', "
          + "starting with a LETTER). This guarantees systemd-escape "
          + "round-trips identically, that tap/interface names "
          + "stay within IFNAMSIZ, and that tooling treating the "
          + "leading digit as a numeric index (e.g. `ip link show`) "
          + "doesn't mis-resolve the name.";
      }
      {
        assertion = !(reservedVmName name);
        message = "nixling.vms.${name}: 'launcher' is reserved for "
          + "the polkit-launcher group (nixling-launcher); pick "
          + "another name.";
      }
      {
        assertion = !(reservedVmPrefix name);
        message = "nixling.vms.${name}: names starting with 'sys-' "
          + "are reserved for nixling's auto-declared system VMs "
          + "(e.g. sys-<env>-net for each declared env, plus "
          + "nixling.observability.vmName when observability is "
          + "enabled). Rename this VM or — if it's intentionally a "
          + "system VM — register it via nixling.envs.<env>.netName "
          + "instead.";
      }
      {
        assertion = !(vm.enable && vm.observability.enable && !obsCfg.enable);
        message = "VM ${name} has observability.enable = true but nixling.observability.enable is false. Per-VM observability requires the framework-level toggle (auto-declares the sys-obs-stack telemetry sink).";
      }
      {
        assertion = !(vm.enable && vm.audit.enable && !vm.observability.enable);
        message = "nixling.vms.${name}.audit.enable requires observability.enable on the same VM";
      }
      {
        # v1.1-P2 primary error path (per ADR 0015): the
        # `mkRemovedOptionModule` shim approach is incompatible
        # with `attrsOf submodule` semantics (no `assertions` option
        # at the per-submodule layer). The supervisor-removal
        # friendly message is therefore emitted by this top-level
        # assertion, which fires whenever any per-VM `vm` attrset
        # carries a `supervisor` attribute.
        assertion = !(vm.enable && (vm ? supervisor));
        message = ''
          nixling.vms.${name}.supervisor was removed in v1.1
          per ADR 0015 (daemon-only clean break). The v1.0
          daemon-only end-state makes "nixlingd" the only valid
          supervisor; v1.1 completes the migration by deleting
          the option entirely. Remove every "supervisor = ..."
          line from your consumer flake.

          The daemon-only path is the default and only path; see
          docs/how-to/migrate-nixling-v1-0-to-v1-1.md.
        '';
      }
      {
        # Phase 2b: `nixling.vms.<name>.entra-id.*` was removed; the
        # option is a kept-but-internal stub so legacy assignments
        # land here instead of producing a cryptic
        # "option does not exist" error from the module system.
        assertion = vm.entra-id == { };
        message = ''
          nixling.vms.${name}.entra-id.* was removed in Phase 2b.
          Himmelblau / Microsoft Entra ID support has moved out of
          the nixling framework into the sibling
          `vicondoa/nixos-entra-id` flake. To migrate:

            inputs.nixos-entra-id.url =
              "github:vicondoa/nixos-entra-id";

            nixling.vms.${name}.config.imports = [
              inputs.nixos-entra-id.nixosModules.default
            ];

            # Move each `nixling.vms.${name}.entra-id.<key>` setting
            # into the VM's guest config under the sibling module's
            # `services.entra-id.<key>` (or whatever attribute path
            # the sibling module declares — see its README).
            nixling.vms.${name}.config.services.entra-id = {
              enable    = true;
              domain    = [ "contoso.com" ];
              # ...
            };

          See CHANGELOG.md (Phase 2b: Removed) and the
          nixos-entra-id README for the full migration recipe.
        '';
      }
      {
        # v0.1.6 SWArch-M9: graphics VMs CANNOT be autostart. The
        # `nixling@<vm>` wrapper template starts `microvm@<vm>`,
        # which is the upstream microvm.nix runner — but graphics
        # VMs run cloud-hypervisor via the `nixling-<vm>-gpu`
        # sidecar (which replaces the upstream runner). The sidecar
        # binds to /run/user/<wayland-uid>/wayland-0, which only
        # exists in a live user session, so it MUST be launched
        # interactively from a Plasma terminal via `nixling up <vm>`.
        # An autostart=true graphics VM would silently boot through
        # the wrong path and never attach to the host compositor.
        assertion = !(vm.enable && vm.graphics.enable && vm.autostart);
        message = ''
          nixling.vms.${name}: graphics.enable = true is incompatible
          with autostart = true. Graphics VMs are launched by the
          nixling CLI through nixling-${name}-gpu.service, which
          binds to /run/user/<uid>/wayland-0 — that socket only
          exists in a live user session. The systemd boot path
          would start microvm@${name}.service (the upstream runner)
          bypassing the GPU sidecar entirely, and the VM would have
          no display.

          Set `nixling.vms.${name}.autostart = false` and launch
          the VM interactively via `nixling up ${name}` from a
          Plasma terminal (or wire it to your Plasma session's
          autostart entries).
        '';
      }
    ])
    cfg.vms;

  envAssertions = lib.mapAttrsToList
    (name: env:
      let
        cidr = env.uplinkSubnet;
        host = subnetIp cidr 1;
        net = subnetIp cidr 2;
      in [
        {
          assertion = envNameOk name;
          message = "nixling.envs.${name}: env name must match the "
            + "regex ^[a-z][a-z0-9-]*$ (lowercase alnum + '-', "
            + "starting with a LETTER). This guarantees systemd-escape "
            + "and `br-<env>-lan` / `<env>-l<index>` interface names "
            + "are well-formed and unambiguous to `ip link`.";
        }
        {
          assertion = cidrContains cidr host && cidrContains cidr net;
          message = "env ${name}: uplinkSubnet ${cidr} cannot be materialized — derived host IP ${host} and net IP ${net} are outside the CIDR.";
        }
      ])
    cfg.envs;

  observabilityAssertions =
    map
      (collision: {
        assertion = false;
        message = "Vsock CID collision: VMs ${collision.vm1}, ${collision.vm2} both compute to CID ${toString collision.cid}. Adjust nixling.vms.<vm>.index in the affected env or rename one VM.";
      })
      workloadObsCidCollisions
    ++ lib.optional (obsCfg.enable && reservedObsCidUsers != [ ]) {
      assertion = false;
      message = ''
        Vsock CID 1000 is reserved for nixling.observability.vmName (${obsCfg.vmName}), but VMs ${lib.concatStringsSep ", " reservedObsCidUsers} also compute to CID 1000. Adjust nixling.vms.<vm>.index in the affected env or rename one VM.
      '';
    };

  # Site-level assertions (Phase 2b — host-specific bias was extracted
  # into `nixling.site.*`; these checks make sure the consumer actually
  # set the options the framework needs for the features it enables).
  needsWaylandUser =
    lib.any
      (vm: vm.enable && (vm.graphics.enable || vm.audio.enable))
      (lib.attrValues cfg.vms);

  siteAssertions =
    [
      {
        assertion = toString cfg.site.stateDir == "/var/lib/nixling";
        message = ''
          nixling.site.stateDir is reserved but not fully threaded yet.
          Leave it at /var/lib/nixling for now; overriding it would
          split host-side state across inconsistent roots.
        '';
      }
      {
        assertion = toString cfg.store.stateDir == "/var/lib/nixling/vms";
        message = ''
          nixling.store.stateDir is reserved but not fully threaded yet.
          Leave it at /var/lib/nixling/vms for now; overriding it would
          desynchronise the manifest, CLI, and per-VM runtime state.
        '';
      }
    ]
    ++
    # If any VM uses graphics or audio, the host MUST point at a
    # Wayland user — that's the user whose XDG_RUNTIME_DIR the GPU /
    # audio sidecars bind into.
    lib.optional needsWaylandUser {
      assertion = cfg.site.waylandUser != null;
      message = ''
        nixling: at least one declared VM has graphics.enable = true
        or audio.enable = true, but `nixling.site.waylandUser` is
        unset (null). The GPU + audio sidecars need a Wayland user
        so they can find the host compositor's pipewire-0 / wayland-0
        sockets under /run/user/<uid>/.

        Set the option to the Plasma / sway / Hyprland user that
        invokes `nixling up <vm>`:

          nixling.site.waylandUser = "alice";

        For headless deployments with no graphics or audio VMs,
        leave the option as null and disable the offending toggles.
      '';
    }
    # If `waylandUser` is set, the corresponding system user must
    # actually exist. Otherwise the sidecar templates render with a
    # dangling /run/user/<unset-uid>/ path and the eval-time error
    # ("…has no attribute uid") is opaque.
    ++ lib.optional (cfg.site.waylandUser != null) {
      assertion = config.users.users ? "${cfg.site.waylandUser}";
      message = ''
        nixling.site.waylandUser = "${cfg.site.waylandUser}" but
        config.users.users.${cfg.site.waylandUser} is not declared.

        Declare the user in your top-level NixOS config:

          users.users.${cfg.site.waylandUser} = {
            isNormalUser = true;
            uid = 1000;            # match your real Plasma user
            extraGroups = [ "wheel" "video" "audio" ];
          };

        nixling references this user's UID to locate
        /run/user/<uid>/{wayland-0,pipewire-0} from the GPU and
        audio sidecars.
      '';
    }
    # launcherUsers entries must reference real users (same rationale
    # as waylandUser — extraGroups merging on a non-existent user is
    # a silent no-op).
    ++ map
      (u: {
        assertion = config.users.users ? "${u}";
        message = ''
          nixling.site.launcherUsers contains "${u}" but no
          users.users.${u} is declared. The nixling-launcher group
          is added to that user via extraGroups; non-existent users
          silently no-op.
        '';
      })
      cfg.site.launcherUsers;

  # Validate every authorized-key entry (site-level + per-VM).
  siteAuthorizedKeyAssertions =
    validateAuthorizedKeys "nixling.site.userAuthorizedKeys"
      cfg.site.userAuthorizedKeys;

  perVmAuthorizedKeyAssertions = lib.flatten (lib.mapAttrsToList
    (name: vm:
      validateAuthorizedKeys
        "nixling.vms.${name}.userAuthorizedKeys"
        vm.userAuthorizedKeys)
    cfg.vms);
in
{
  assertions = lib.flatten (
    vmAssertions
    ++ envAssertions
    ++ observabilityAssertions
    ++ siteAssertions
    ++ siteAuthorizedKeyAssertions
    ++ perVmAuthorizedKeyAssertions
  );

  # v1.1-P4: deprecation warning when a consumer flake still sets
  # nixling.daemonExperimental.enable. The option is now obsolete
  # because v1.1 promotes the broker socket/service to default-on
  # (per ADR 0015 daemon-only clean break + the v1.1-P4 TDD row).
  # We emit a warnings entry (NOT an assertion failure — leaving
  # the option set must not block eval, only notify) so operators
  # get a clear migration cue in nixos-rebuild output.
  warnings =
    lib.optional
      (options.nixling.daemonExperimental.enable.isDefined or false)
      "nixling.daemonExperimental.enable is obsolete in v1.1; remove this option from your consumer flake because the broker socket/service are enabled by default. Leaving it set has no effect.";
}
