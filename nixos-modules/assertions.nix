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
{ config, lib, ... }:

let
  cfg = config.nixling;

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

  # Auto-declared system VMs (added to cfg.vms by network.nix) have
  # names of the form `sys-<env>-net`. We must NOT reject those for
  # the `sys-` reserved-prefix rule, so derive the set of auto-system
  # VM names from `nixling.envs.<env>.netName` (default
  # `sys-${env}-net`) and treat them as allowed.
  autoSysVmNames =
    lib.mapAttrsToList
      (envName: env: env.netName or "sys-${envName}-net")
      cfg.envs;

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
          + "(e.g. sys-<env>-net for each declared env). Rename "
          + "this VM or — if it's intentionally a system VM — "
          + "register it via nixling.envs.<env>.netName instead.";
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
    ])
    cfg.vms;

  envAssertions = lib.mapAttrsToList
    (name: _env: [
      {
        assertion = envNameOk name;
        message = "nixling.envs.${name}: env name must match the "
          + "regex ^[a-z][a-z0-9-]*$ (lowercase alnum + '-', "
          + "starting with a LETTER). This guarantees systemd-escape "
          + "and `br-<env>-lan` / `<env>-l<index>` interface names "
          + "are well-formed and unambiguous to `ip link`.";
      }
    ])
    cfg.envs;

  # Site-level assertions (Phase 2b — host-specific bias was extracted
  # into `nixling.site.*`; these checks make sure the consumer actually
  # set the options the framework needs for the features it enables).
  needsWaylandUser =
    lib.any
      (vm: vm.enable && (vm.graphics.enable || vm.audio.enable))
      (lib.attrValues cfg.vms);

  siteAssertions =
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
    ++ siteAssertions
    ++ siteAuthorizedKeyAssertions
    ++ perVmAuthorizedKeyAssertions
  );
}
