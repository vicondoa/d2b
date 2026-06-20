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
{ config, lib, options, pkgs, ... }:

let
  cfg = config.nixling;
  obsCfg = cfg.observability;
  obsVsockCid = 1000;
  u32Max = 4294967295;
  nl = import ./lib.nix { inherit lib; };
  inherit (nl) parseCidr subnetIp volumeSerialIssues;

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
  # either a path (a.pub file on disk, e.g. /etc/nixos/keys/foo.pub)
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
  autoGatewayVmNames =
    lib.mapAttrsToList
      (_: gw: gw.vmName)
      (lib.filterAttrs (_: gw: gw.enable) cfg.gateways);
  enabledGateways = lib.mapAttrsToList
    (name: gw: { inherit name gw; })
    (lib.filterAttrs (_: gw: gw.enable) cfg.gateways);
  enabledGatewayNames = map (gateway: gateway.name) enabledGateways;
  enabledGatewayRealms = map (gateway: gateway.gw.realm) enabledGateways;
  enabledGatewayEnvs = map (gateway: gateway.gw.env) enabledGateways;
  enabledGatewayVmNames = map (gateway: gateway.gw.vmName) enabledGateways;
  duplicateValues = values:
    let
      countValue = value: lib.length (lib.filter (candidate: candidate == value) values);
    in
    lib.unique (lib.filter (value: countValue value > 1) values);
  duplicateGatewayRealms = duplicateValues enabledGatewayRealms;
  duplicateGatewayEnvs = duplicateValues enabledGatewayEnvs;
  duplicateGatewayVmNames = duplicateValues enabledGatewayVmNames;

  autoSysVmNames =
    (lib.mapAttrsToList
      (envName: env: env.netName or "sys-${envName}-net")
      cfg.envs)
    ++ lib.optional obsCfg.enable obsCfg.vmName
    ++ autoGatewayVmNames;

  secretShaped = s:
    lib.hasInfix "SharedAccessKey" s
    || lib.hasInfix "Endpoint=sb://" s
    || lib.hasInfix "AccountKey=" s
    || lib.hasInfix "PRIVATE KEY" s
    || lib.hasInfix "BEGIN " s;

  underStateDir = s:
    lib.hasPrefix "${toString cfg.site.stateDir}/" s;

  pathComponents = s:
    lib.filter (part: part != "") (lib.splitString "/" s);

  hasParentTraversal = s:
    builtins.elem ".." (pathComponents s);

  hasTrailingSlash = s:
    s != "/" && lib.hasSuffix "/" s;

  absoluteRuntimePathUnder = root: s:
    lib.hasPrefix "/" s
    && lib.hasPrefix "${toString root}/" s
    && !(hasParentTraversal s)
    && !(hasTrailingSlash s);

  gatewayPathAssertions =
    lib.flatten (lib.mapAttrsToList
      (name: gw:
        let
          paths = {
            stateDir = gw.stateDir;
            credentialPath = gw.credentialPath;
          };
        in
        lib.mapAttrsToList
          (field: value: {
            assertion =
              absoluteRuntimePathUnder cfg.site.stateDir value
              && !(lib.hasPrefix "/nix/store/" value)
              && !(secretShaped value);
            message = ''
              nixling.gateways.${name}.${field} must be an absolute runtime
              path under nixling.site.stateDir, must not contain `..` path
              components or a trailing slash, and must not contain inline
              secret-shaped material. Put plaintext credentials in the gateway
              guest's sealed runtime state, not in the host Nix configuration.
            '';
          })
          paths)
      (lib.filterAttrs (_: gw: gw.enable) cfg.gateways));

  gatewayCredentialPathAssertions =
    lib.mapAttrsToList
      (name: gw: {
        assertion = absoluteRuntimePathUnder gw.stateDir gw.credentialPath;
        message = ''
          nixling.gateways.${name}.credentialPath must live under
          nixling.gateways.${name}.stateDir so the gateway credential envelope
          stays inside the gateway runtime-state boundary.
        '';
      })
      (lib.filterAttrs (_: gw: gw.enable) cfg.gateways);

  gatewayStateBoundaryAssertions =
    lib.mapAttrsToList
      (name: gw: {
        assertion =
          gw.stateDir != toString cfg.store.stateDir
          && !(underStateDir gw.stateDir && lib.hasPrefix "${toString cfg.store.stateDir}/" gw.stateDir);
        message = ''
          nixling.gateways.${name}.stateDir must not live under
          nixling.store.stateDir. Gateway credential state is distinct from
          per-VM runtime state and has different host/guest ownership.
        '';
      })
      (lib.filterAttrs (_: gw: gw.enable) cfg.gateways);

  gatewayCoordinateAssertions =
    lib.flatten (lib.mapAttrsToList
      (name: gw:
        let
          coordinates = lib.filter (v: v != null) [
            gw.relay.namespace
            gw.relay.entity
            gw.aca.endpoint
            gw.aca.subscription
            gw.aca.resourceGroup
            gw.aca.sandboxGroup
            gw.aca.region
            gw.aca.diskImageId
            gw.aca.image
            gw.aca.diskName
            gw.aca.managedIdentityResourceId
            gw.display.waypipeSocket
          ];
        in
        lib.imap0
          (i: value: {
            assertion = !(secretShaped value);
            message = ''
              nixling.gateways.${name} coordinate #${toString i} looks like
              inline credential material. Gateway options may carry non-secret
              endpoint names only.
            '';
          })
          coordinates)
      (lib.filterAttrs (_: gw: gw.enable) cfg.gateways));

  gatewayEntrypointAssertions = [
    {
      assertion = !(builtins.elem "local" enabledGatewayRealms);
      message = ''
        nixling.gateways may not declare realm `local`: the local realm
        entrypoint is always host-resident so the local fast path remains
        unambiguous.
      '';
    }
    {
      assertion = duplicateGatewayRealms == [ ];
      message = ''
        nixling.gateways must declare at most one gateway-backed realm
        entrypoint per realm path. Duplicate realm path(s): ${
          lib.concatStringsSep ", " duplicateGatewayRealms
        }.
      '';
    }
    {
      assertion = duplicateGatewayVmNames == [ ];
      message = ''
        nixling.gateways must declare a separate gateway guest per
        gateway-backed realm. Duplicate gateway VM name(s): ${
          lib.concatStringsSep ", " duplicateGatewayVmNames
        }.
      '';
    }
    {
      assertion = duplicateGatewayEnvs == [ ];
      message = ''
        nixling.gateways must not place multiple gateway-backed realms on the
        same nixling.envs L2 segment. Shared gateway env(s): ${
          lib.concatStringsSep ", " duplicateGatewayEnvs
        }.
      '';
    }
  ];

  gatewayDaemonAssertions = lib.optional (enabledGatewayNames != [ ]) {
    assertion = cfg.daemonExperimental.enable;
    message = ''
      nixling.gateways requires nixling.daemonExperimental.enable = true. The
      gateway guest is supervised by the daemon control-plane package plumbing
      and has no legacy service or bash fallback.
    '';
  };

  # Systemd-escape identity regex (lower-case alnum and `-`, must
  # start with a LETTER). `^[a-z][a-z0-9-]*$` deliberately excludes
  #   * `.` (dots — systemd-escape would turn them into `\x2e`)
  #   * `_` (underscores — same)
  #   * `@` (would collide with template-instance separator)
  #   * `/` (path separator)
  #   * uppercase (NixOS option names are case-sensitive but
  #     downstream tooling like `systemctl --type=service` is not
  #     consistent; lower-case avoids the foot-gun)
  #   * leading `-` (looks like a flag)
  #   * leading digit (a numeric-prefixed VM/env name like `42web`
  #     produces unit names such as `nixling@42web.service` and tap
  #     names like `42web-l10` which are technically legal but trip up
  #     tooling that treats the leading digit as a numeric argument —
  #     e.g. `ip link show 42web-l10` resolves to the interface at
  #     index 42 first. Requiring a leading letter matches
  #     systemd-escape best practices and avoids the ambiguity.)
  vmNameOk = name:
    builtins.match "^[a-z][a-z0-9-]*$" name != null;

  # Reserved single-name: `launcher` is taken by the polkit-launcher
  # group (`nixling`) singleton. A VM named `launcher` would
  # produce `nixling-gpu` etc. users that collide with the
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
  # Same leading-letter restriction as vmNameOk
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

  vmDefinitionsFor = name:
    lib.filter
      (d: builtins.isAttrs d.value && builtins.hasAttr name d.value)
      options.nixling.vms.definitionsWithLocations;

  vmSubOptionDefined = name: optionName:
    lib.any
      (d:
        let value = d.value.${name};
        in builtins.isAttrs value && builtins.hasAttr optionName value)
      (vmDefinitionsFor name);

  vmVsockCidPairs = lib.mapAttrsToList
    (name: _vm: {
      inherit name;
      cid = config.nixling.manifest.${name}.observability.vsockCid;
    })
    (nl.normalNixosVms cfg.vms);

  vmVsockCidGroups = lib.groupBy
    (pair: toString pair.cid)
    vmVsockCidPairs;

  collidingVmVsockCidGroups = lib.filterAttrs
    (_: pairs: builtins.length pairs > 1)
    vmVsockCidGroups;

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

  vmVsockCidCollisions =
    lib.flatten (map mkCidCollisionPairs (lib.attrValues collidingVmVsockCidGroups));

  invalidVmVsockCidUsers = lib.filter
    (pair: builtins.elem pair.cid [ 0 1 2 u32Max ])
    vmVsockCidPairs;

  reservedObsCidUsers = map (pair: pair.name)
    (lib.filter
      (pair: pair.cid == obsVsockCid && !(obsCfg.enable && pair.name == obsCfg.vmName))
      vmVsockCidPairs);

  vmVsockSocketPairs = lib.mapAttrsToList
    (name: _vm: {
      inherit name;
      socket = config.nixling.manifest.${name}.observability.vsockHostSocket;
    })
    (nl.normalNixosVms cfg.vms);

  socketPathTooLong = path: builtins.stringLength path > 107;

  tooLongVmVsockSockets = lib.filter
    (pair:
      socketPathTooLong pair.socket
      || socketPathTooLong "${pair.socket}_${toString nl.observabilityOtlpVsockPort}"
      || socketPathTooLong "${pair.socket}_${toString nl.guestControlVsockPort}")
    vmVsockSocketPairs;

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
          + "the polkit-launcher group (nixling); pick "
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
        message = "VM ${name} has observability.enable = true but nixling.observability.enable is false. Per-VM observability requires the framework-level toggle (auto-declares the sys-obs telemetry sink).";
      }
      {
        assertion = !(vm.enable && vm.audit.enable && !vm.observability.enable);
        message = "nixling.vms.${name}.audit.enable requires observability.enable on the same VM";
      }
      {
        assertion = !(vm.enable && vm.graphics.videoSidecar && !vm.graphics.enable);
        message = ''
          nixling.vms.${name}.graphics.videoSidecar requires graphics.enable = true.
          Enable graphics for this VM or disable graphics.videoSidecar.
        '';
      }
      {
        assertion = !(vm.enable && vm.graphics.videoNvidiaDecode && !vm.graphics.videoSidecar);
        message = ''
          nixling.vms.${name}.graphics.videoNvidiaDecode requires graphics.videoSidecar = true.
          Enable the video sidecar or disable graphics.videoNvidiaDecode.
        '';
      }
      {
        assertion = !(vm.enable && vm.graphics.virglVideo && !vm.graphics.enable);
        message = ''
          nixling.vms.${name}.graphics.virglVideo requires graphics.enable = true.
          Enable graphics for this VM or disable graphics.virglVideo.
        '';
      }
      {
        # Xwayland is intentionally unsupported during the Wayland-only
        # migration to wl-cross-domain-proxy + nixling-wayland-filter.
        # The previous wayland-proxy-virtwl had --xwayland-binary support;
        # wl-cross-domain-proxy is a plain cross-domain transport and
        # carries no Xwayland helper. A standalone host-side Xwayland
        # proxy is tracked as future work.
        assertion = !(vm.enable && vm.graphics.xwayland.enable);
        message = ''
          nixling.vms.${name}.graphics.xwayland.enable = true is not
          supported in this release.

          The guest Wayland transport has been replaced with
          wl-cross-domain-proxy, which does not include an Xwayland
          helper. X11 support is tracked as future work.

          Remediation: set graphics.xwayland.enable = false (the default).
        '';
      }
      {
        # primary error path (per ADR 0015): the
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
        # `nixling.vms.<name>.entra-id.*` was removed; the
        # option is a kept-but-internal stub so legacy assignments
        # land here instead of producing a cryptic
        # "option does not exist" error from the module system.
        assertion = vm.entra-id == { };
        message = ''
          nixling.vms.${name}.entra-id.* was removed.
          Himmelblau / Microsoft Entra ID support has moved out of
          the nixling framework into the sibling
          `vicondoa/entrablau.nix` flake. To migrate:

            inputs.entrablau.url =
              "github:vicondoa/entrablau.nix";

            nixling.vms.${name}.config.imports = [
              inputs.entrablau.nixosModules.default
            ];

            # Move each `nixling.vms.${name}.entra-id.<key>` setting
            # into the VM's guest config under the sibling module's
            # `entrablau.<key>` option tree.
            nixling.vms.${name}.config.entrablau = {
              enable    = true;
              domain    = [ "contoso.com" ];
              # ...
            };

          See CHANGELOG.md and the
          entrablau README for the full migration recipe.
        '';
      }
      {
        # `nixling.vms.<name>.guest.exec.allowRoot` was removed:
        # guest-control exec now ALWAYS runs as the VM's workload
        # user (`ssh.user`) inside a PAM login session, never root.
        # The option is a kept-but-internal stub (options-vms.nix) so
        # legacy assignments land on this friendly message instead of
        # a cryptic "option does not exist" module-system error.
        assertion = !(vm.enable && vm.guest.exec.allowRoot);
        message = ''
          nixling.vms.${name}.guest.exec.allowRoot was removed.
          Guest-control exec now always runs as the VM's workload
          user (`ssh.user`) inside a PAM login session — never as
          root. There is no root-exec mode. Remove
          `guest.exec.allowRoot = ...;`; to run a command as root,
          elevate with `sudo` inside the exec session.
        '';
      }
      {
        # `nixling.vms.<name>.guest.exec.users` was removed: there is
        # no per-VM exec user allowlist; exec always targets the
        # single workload user (`ssh.user`). Kept-but-internal stub.
        assertion = !(vm.enable && vm.guest.exec.users != [ ]);
        message = ''
          nixling.vms.${name}.guest.exec.users was removed.
          Guest-control exec now always targets the VM's single
          workload user (`ssh.user`); there is no per-VM exec user
          allowlist. Remove `guest.exec.users = [ ... ];`.
        '';
      }
      {
        # Graphics VMs CANNOT be autostart. The
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

  qemuMediaAssertions = lib.flatten (lib.mapAttrsToList
    (name: vm:
      let
        mediaSources =
          (lib.optional (vm.qemuMedia.source != null) {
            slot = "boot";
            source = vm.qemuMedia.source;
          })
          ++ (lib.flatten (lib.mapAttrsToList
            (slotName: slot:
              lib.optional (slot.source != null) {
                slot = slotName;
                source = slot.source;
              })
            vm.qemuMedia.removableSlots));
        declaredMediaRefs =
          map (entry: entry.source.ref)
            (lib.filter (entry: entry.source.ref != null) mediaSources);
        duplicateMediaRefs =
          lib.unique (lib.filter
            (ref: lib.length (lib.filter (other: other == ref) declaredMediaRefs) > 1)
            declaredMediaRefs);
        sourceAssertions = lib.flatten (map
          (entry:
            let
              source = entry.source;
              sourceName = "nixling.vms.${name}.qemuMedia.${if entry.slot == "boot" then "source" else "removableSlots.${entry.slot}.source"}";
              isPhysical = source.kind == "physical-usb";
              isImage = source.kind == "image-file";
            in [
              {
                assertion = (!isPhysical) || source.ref != null;
                message = ''
                  ${sourceName}: kind = "physical-usb" requires an opaque `ref`.
                  Discover live runtime selectors with `nixling usb probe`;
                  do not place bus IDs, serials, or paths in Nix config.
                '';
              }
              {
                assertion = (!isPhysical) || source.path == null;
                message = ''
                  ${sourceName}: kind = "physical-usb" must not set `path`.
                  Physical USB remains opaque-ref based so raw device identity
                  stays out of Nix-store-backed artifacts.
                '';
              }
              {
                assertion = (!isImage) || source.path != null;
                message = ''
                  ${sourceName}: kind = "image-file" requires an absolute
                  `path`, for example
                  `/var/lib/nixling/images/${name}-${entry.slot}.img`.
                '';
              }
              {
                assertion = (!isImage) || source.format == "raw";
                message = ''
                  ${sourceName}: kind = "image-file" supports only
                  `format = "raw"`; QEMU format auto-probing is never used.
                '';
              }
              {
                assertion =
                  (!isImage)
                  || (source.path != null
                    && lib.hasPrefix "/" source.path
                    && !(lib.hasInfix "\n" source.path)
                    && !(lib.hasInfix "\r" source.path));
                message = ''
                  ${sourceName}: image-file `path` must be an absolute
                  single-line host path.
                '';
              }
            ])
          mediaSources);
      in
      lib.optionals (vm.enable && vm.runtime.kind == "qemu-media") ([
        {
          assertion = vm.env != null;
          message = ''
            nixling.vms.${name}: runtime.kind = "qemu-media" requires
            `env` in this foundational implementation so networking can be
            derived without evaluating a guest NixOS configuration.
          '';
        }
        {
          assertion = pkgs.stdenv.hostPlatform.system == "x86_64-linux";
          message = ''
            nixling.vms.${name}: runtime.kind = "qemu-media" is currently
            supported only on x86_64-linux because its QEMU argv uses the
            q35/virtio-vga device model.
          '';
        }
        {
          assertion = vm.qemuMedia.source != null;
          message = ''
            nixling.vms.${name}: runtime.kind = "qemu-media" requires
            `qemuMedia.source` in this implementation. Declare either a
            physical-usb opaque ref discovered with `nixling usb probe`, or a
            direct image-file source configured in Nix.
          '';
        }
        {
          assertion = !(builtins.hasAttr "boot" vm.qemuMedia.removableSlots);
          message = ''
            nixling.vms.${name}: qemu-media removable slot name `boot` is
            reserved for `qemuMedia.source`. Use a different
            `qemuMedia.removableSlots.<name>` such as `backup`.
          '';
        }
        {
          assertion = duplicateMediaRefs == [ ];
          message = ''
            nixling.vms.${name}: qemu-media refs must be unique per VM;
            duplicate opaque ref(s): ${lib.concatStringsSep ", " duplicateMediaRefs}.
          '';
        }
        {
          assertion = vmSubOptionDefined name "index";
          message = ''
            nixling.vms.${name}: runtime.kind = "qemu-media" requires an
            explicit `index` in this foundational implementation. Set
            `nixling.vms.${name}.index` to this VM's env slot.
          '';
        }
        {
          assertion = !(vmSubOptionDefined name "config");
          message = ''
            nixling.vms.${name}: runtime.kind = "qemu-media" must not define
            `config`. qemu-media VMs are external media runtimes and skip the
            per-VM NixOS evaluator.
          '';
        }
        {
          assertion = vm.guestConfigFile == null;
          message = ''
            nixling.vms.${name}: runtime.kind = "qemu-media" is incompatible
            with guestConfigFile because there is no nixling-managed guest
            NixOS configuration to sync.
          '';
        }
        {
          assertion =
            !vm.guest.control.enable
            && vm.guest.control.auth.tokenFile == null
            && !vm.guest.exec.enable
            && !vm.guest.exec.allowRoot
            && vm.guest.exec.users == [ ];
          message = ''
            nixling.vms.${name}: runtime.kind = "qemu-media" is incompatible
            with guest-control and guest exec options. Disable
            guest.control.* and guest.exec.* for this manual-only runtime.
          '';
        }
        {
          assertion =
            vm.ssh.user == null
            && vm.ssh.keyPath == null
            && vm.userAuthorizedKeys == [ ]
            && !vm.sudo;
          message = ''
            nixling.vms.${name}: runtime.kind = "qemu-media" is incompatible
            with nixling-managed SSH, sudo, and per-VM authorized-key options.
          '';
        }
        {
          assertion = !(vmSubOptionDefined name "homeManager");
          message = ''
            nixling.vms.${name}: runtime.kind = "qemu-media" is incompatible
            with home-manager guest configuration.
          '';
        }
        {
          assertion = !vm.audit.enable;
          message = ''
            nixling.vms.${name}: runtime.kind = "qemu-media" is incompatible
            with guest audit forwarding.
          '';
        }
        {
          assertion = !vm.observability.enable;
          message = ''
            nixling.vms.${name}: runtime.kind = "qemu-media" is incompatible
            with guest observability.
          '';
        }
        {
          assertion = !vm.usbip.yubikey && vm.usbip.busids == [ ];
          message = ''
            nixling.vms.${name}: runtime.kind = "qemu-media" is incompatible
            with nixling USBIP/YubiKey passthrough declarations.
          '';
        }
        {
          assertion =
            !vm.graphics.enable
            && !vm.graphics.crossDomainTrusted
            && !vm.graphics.xwayland.enable
            && !vm.graphics.videoSidecar
            && !vm.graphics.videoNvidiaDecode
            && !vm.graphics.virglVideo
            && !vm.graphics.renderNodeOnly
            && vm.graphics.niriBorderColor == null
            && !vm.graphics.waylandFilter.debugLogging
            && !vm.graphics.waylandFilter.byteLogging
            && vm.graphics.waylandFilter.denyGlobals == [ ]
            && vm.graphics.waylandFilter.allowGlobals == [ ]
            && vm.graphics.waylandFilter.maxVersions == { }
            && vm.graphics.waylandFilter.dmabufAllow == [ ]
            && vm.graphics.waylandFilter.dmabufDeny == [ ];
          message = ''
            nixling.vms.${name}: runtime.kind = "qemu-media" is incompatible
            with nixling graphics options.
          '';
        }
        {
          assertion = !vm.tpm.enable;
          message = ''
            nixling.vms.${name}: runtime.kind = "qemu-media" is incompatible
            with nixling-managed TPM state.
          '';
        }
        {
          assertion =
            !vm.audio.enable
            && !vm.audio.allowMicByDefault
            && !vm.audio.allowSpeakerByDefault
            && vm.audio.users == [ ];
          message = ''
            nixling.vms.${name}: runtime.kind = "qemu-media" is incompatible
            with nixling audio sidecar options.
          '';
        }
        {
          assertion = !vm.autostart;
          message = ''
            nixling.vms.${name}: runtime.kind = "qemu-media" is manual-only in
            this foundational implementation; `autostart = true` is not
            supported until unattended QMP continuation is available.
          '';
        }
      ] ++ sourceAssertions))
    cfg.vms);

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

  vsockAssertions =
    map
      (collision: {
        assertion = false;
        message = "Vsock CID collision: VMs ${collision.vm1}, ${collision.vm2} both compute to CID ${toString collision.cid}. Adjust nixling.vms.<vm>.index in the affected env or rename one VM.";
      })
      vmVsockCidCollisions
    ++ map
      (pair: {
        assertion = false;
        message = ''
          nixling.vms.${pair.name}: derived Cloud Hypervisor vsock CID
          ${toString pair.cid} is reserved by Linux/AF_VSOCK. Rename the VM or
          adjust env/index so nixling derives a guest CID outside 0, 1, 2, and
          ${toString u32Max}.
        '';
      })
      invalidVmVsockCidUsers
    ++ lib.optional (obsCfg.enable && reservedObsCidUsers != [ ]) {
      assertion = false;
      message = ''
        Vsock CID 1000 is reserved for nixling.observability.vmName (${obsCfg.vmName}), but VMs ${lib.concatStringsSep ", " reservedObsCidUsers} also compute to CID 1000. Adjust nixling.vms.<vm>.index in the affected env or rename one VM.
      '';
    }
    ++ map
      (pair: {
        assertion = false;
        message = ''
          nixling.vms.${pair.name}: Cloud Hypervisor vsock socket path is too
          long for Linux AF_UNIX after port suffixes are considered:
          ${pair.socket}. Shorten nixling.site.stateDir or the VM name.
        '';
      })
      tooLongVmVsockSockets;

  # Site-level assertions (host-specific bias was extracted
  # into `nixling.site.*`; these checks make sure the consumer actually
  # set the options the framework needs for the features it enables).
  needsWaylandUser =
    lib.any
      (vm: vm.enable && (vm.graphics.enable || vm.audio.enable || vm.runtime.kind == "qemu-media"))
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
    # If any VM uses graphics, audio, or qemu-media, the host MUST point
    # at a Wayland user — that's the user whose XDG_RUNTIME_DIR holds the
    # compositor / PipeWire sockets those host-side processes need.
    lib.optional needsWaylandUser {
      assertion = cfg.site.waylandUser != null;
      message = ''
        nixling: at least one declared VM has graphics.enable = true,
        audio.enable = true, or runtime.kind = "qemu-media", but
        `nixling.site.waylandUser` is unset (null). The host-side
        display/audio processes need a Wayland user so they can find the
        host compositor's pipewire-0 / wayland-0 sockets under
        /run/user/<uid>/.

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
          users.users.${u} is declared. The nixling group
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

  volumeSerialAssertions = lib.flatten (lib.mapAttrsToList
    (name: vm:
      let
        microvm = nl.vmRunner config name;
        serialIssues = volumeSerialIssues microvm.volumes;
      in
      lib.optionals (vm.enable && microvm.volumes != [ ]) [
        {
          assertion = serialIssues.duplicates == [ ];
          message = ''
            nixling.vms.${name}.config.microvm.volumes derives duplicate virtio
            disk serial(s): ${lib.concatStringsSep ", " serialIssues.duplicates}. Set explicit
            unique `serial` values on the volume entries.
          '';
        }
        {
          assertion = serialIssues.reserved == [ ];
          message = ''
            nixling.vms.${name}.config.microvm.volumes uses reserved virtio disk
            serial `rootfs`, which is owned by writableStoreOverlay. Set an
            explicit non-reserved `serial`.
          '';
        }
        {
          assertion = serialIssues.tooLong == [ ];
          message = ''
            nixling.vms.${name}.config.microvm.volumes has virtio disk serial(s)
            longer than 20 bytes: ${lib.concatStringsSep ", " serialIssues.tooLong}. Linux
            truncates virtio-blk serials, so guest mounts would not match.
          '';
        }
        {
          assertion = serialIssues.unsafe == [ ];
          message = ''
            nixling.vms.${name}.config.microvm.volumes has unsafe virtio disk
            serial(s): ${lib.concatStringsSep ", " serialIssues.unsafe}. Use
            only [A-Za-z0-9-], start with an alphanumeric character, and avoid
            delimiters such as comma, equals, slash, and control characters.
          '';
        }
      ])
    (nl.normalNixosVms cfg.vms));

  # Containment for the per-VM guest-editable `guestConfigFile`: it may
  # only set guest OS options, never host-owned microvm.* / nixling.*.
  # The namespace-containment check (evalModules over the real nixpkgs
  # NixOS module set, definition-existence; catches imports / generated
  # modules / `_file` spoofing) runs in host.nix's composeVm pass and is
  # read here as `_computed.<name>.guestForbidden`. It is a policy lint,
  # not an eval-time security sandbox (see lib.nix + docs/adr/0024).
  # Only VMs that set a guestConfigFile force that per-VM evaluation, so
  # VMs without one — i.e. every existing consumer — pay nothing here.
  guestConfigContainmentAssertions = lib.mapAttrsToList
    (name: vm:
      let
        guestFile = toString vm.guestConfigFile;
        forbidden = cfg._computed.${name}.guestForbidden or [ ];
      in
      {
        assertion = forbidden == [ ];
        message = ''
          nixling.vms.${name}.guestConfigFile (${guestFile}) may only set
          guest OS options, but it (or a module it imports) sets host-owned
          option(s): ${
            lib.concatStringsSep ", " forbidden
          }. Host-owned microvm.* / nixling.* settings must live in the
          host-owned nixling.vms.${name}.config, which the guest cannot
          edit.
        '';
      })
    (lib.filterAttrs (_: vm: vm.enable && vm.guestConfigFile != null) cfg.vms);
in
{
  assertions = lib.flatten (
    vmAssertions
    ++ qemuMediaAssertions
    ++ envAssertions
    ++ vsockAssertions
    ++ siteAssertions
    ++ siteAuthorizedKeyAssertions
    ++ perVmAuthorizedKeyAssertions
    ++ volumeSerialAssertions
    ++ guestConfigContainmentAssertions
    ++ gatewayPathAssertions
    ++ gatewayCredentialPathAssertions
    ++ gatewayStateBoundaryAssertions
    ++ gatewayCoordinateAssertions
    ++ gatewayEntrypointAssertions
    ++ gatewayDaemonAssertions
  );

  # The daemon-only end state is now the default. Do not warn on the
  # compatibility option here: option-default definitions make
  # `options.<path>.isDefined` true even when consumers do not set it.
  warnings = [ ];
}
