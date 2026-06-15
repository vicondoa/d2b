{ config, lib, pkgs, ... }:

let
  cfg = config.nixling;

  # filter out `target/` dev caches from the source
  # so the Nix copy stays small (workspace target alone is ~17 GB).
  packagesSrc = lib.cleanSourceWith {
    src = ../packages;
    filter = path: type:
      let rel = lib.removePrefix (toString ../packages + "/") (toString path);
      in !(lib.hasInfix "target" rel || lib.hasInfix ".cargo/registry" rel);
  };
  cargoLock = {
    lockFile = ../packages/Cargo.lock;
    outputHashes."wl-proxy-0.1.2" = "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=";
  };

  nixlingdPackage = pkgs.rustPlatform.buildRustPackage {
    pname = "nixlingd";
    version = "0.0.0-bootstrap";
    src = packagesSrc;
    inherit cargoLock;
    cargoBuildFlags = [ "--package" "nixlingd" ];
    doCheck = false;
    # strip the dev-only sccache rustc-wrapper (see
    # host-broker.nix for full rationale). Writing the empty
    # rustc-wrapper into .cargo/config.toml shadows the dev
    # value without touching the parent cargo config.
    postPatch = ''
      mkdir -p .cargo
      cat > .cargo/config.toml <<EOF
[build]
rustc-wrapper = ""
EOF
      rm -f .cargo/rustc-wrapper.sh
    '';
    installPhase = ''
      runHook preInstall
      install -Dm755 target/x86_64-unknown-linux-gnu/release/nixlingd $out/bin/nixlingd 2>/dev/null \
        || install -Dm755 target/release/nixlingd $out/bin/nixlingd
      runHook postInstall
    '';
  };

  # the user-facing CLI is now the Rust nixling crate
  # (packages/nixling). The pre-v1.0 bash CLI was RETIRED in;
  # ships the daemon-native Rust CLI as the only
  # `nixling` binary on the host.
  nixlingCliPackage = pkgs.rustPlatform.buildRustPackage {
    pname = "nixling";
    version = "0.0.0-bootstrap";
    src = packagesSrc;
    inherit cargoLock;
    cargoBuildFlags = [ "--package" "nixling" ];
    doCheck = false;
    postPatch = ''
      mkdir -p .cargo
      cat > .cargo/config.toml <<EOF
[build]
rustc-wrapper = ""
EOF
      rm -f .cargo/rustc-wrapper.sh
    '';
    installPhase = ''
      runHook preInstall
      install -Dm755 target/x86_64-unknown-linux-gnu/release/nixling $out/bin/nixling 2>/dev/null \
        || install -Dm755 target/release/nixling $out/bin/nixling
      runHook postInstall
    '';
  };

  # Small fd-safe activation helper that the host activation snippets
  # call instead of `[ -L ] / [ -f ] / find -type f` shell
  # check-then-act patterns. Lives in nixling-host because it
  # only depends on libc + nix; no IPC; no async runtime.
  nixlingActivationHelperPackage = pkgs.rustPlatform.buildRustPackage {
    pname = "nixling-activation-helper";
    version = "0.0.0-bootstrap";
    src = packagesSrc;
    inherit cargoLock;
    cargoBuildFlags = [ "--package" "nixling-host" "--bin" "nixling-activation-helper" ];
    doCheck = false;
    postPatch = ''
      mkdir -p .cargo
      cat > .cargo/config.toml <<EOF
[build]
rustc-wrapper = ""
EOF
      rm -f .cargo/rustc-wrapper.sh
    '';
    installPhase = ''
      runHook preInstall
      install -Dm755 target/x86_64-unknown-linux-gnu/release/nixling-activation-helper $out/bin/nixling-activation-helper 2>/dev/null \
        || install -Dm755 target/release/nixling-activation-helper $out/bin/nixling-activation-helper
      runHook postInstall
    '';
  };

  nixlingCliShellArtifactsPackage = pkgs.runCommand "nixling-cli-shell-artifacts" { } ''
    install -Dm644 ${../docs/manpages/nixling.1} "$out/share/man/man1/nixling.1"
    ${pkgs.gzip}/bin/gzip -n -c ${../docs/manpages/nixling.1} > "$out/share/man/man1/nixling.1.gz"
    install -Dm644 ${../docs/completions/nixling.bash} "$out/share/bash-completion/completions/nixling"
    install -Dm644 ${../docs/completions/nixling.zsh} "$out/share/zsh/site-functions/_nixling"
    install -Dm644 ${../docs/completions/nixling.fish} "$out/share/fish/vendor_completions.d/nixling.fish"
  '';

  daemonConfigJson = pkgs.writeText "nixlingd-daemon-config.json" (builtins.toJSON {
    publicSocketPath = "/run/nixling/public.sock";
    brokerSocketPath = "/run/nixling/priv.sock";
    stateLockPath = "/run/nixling/daemon.lock";
    locksDir = "/run/nixling/locks";
    daemonUser = "nixlingd";
    daemonGroup = "nixlingd";
    publicSocketGroup = "nixling";
    launcherUsers = cfg.site.launcherUsers;
    adminUsers = cfg.site.adminUsers;
    serverVersion = "0.4.0";
    acceptedClientVersionRange = ">=0.4.0, <0.5.0";
    artifacts = {
      publicManifestPath = "/run/current-system/sw/share/nixling/vms.json";
      bundlePath = "/etc/nixling/bundle.json";
      hostPath = "/etc/nixling/host.json";
      processesPath = "/etc/nixling/processes.json";
      closuresDir = "/etc/nixling/closures";
    };
  });
in
{
  options.nixling.host.usbip.allowlist = lib.mkOption {
    type = lib.types.listOf (lib.types.submodule ({ ... }: {
      options = {
        vendor = lib.mkOption {
          type = lib.types.strMatching "^0x[0-9A-Fa-f]{4}$";
          example = "0x1050";
          description = "Hex USB vendor ID to allow through the trusted USBIP bundle policy.";
        };
        product = lib.mkOption {
          type = lib.types.strMatching "^0x[0-9A-Fa-f]{4}$";
          example = "0x0407";
          description = "Hex USB product ID to allow through the trusted USBIP bundle policy.";
        };
      };
    }));
    default = [ ];
    example = [
      {
        vendor = "0x1050";
        product = "0x0407";
      }
    ];
    description = ''
      Host-wide USBIP vendor:product allowlist copied into the trusted
      host bundle so the broker can refuse devices outside the approved
      hardware set before bind/attach proceeds.
    '';
  };

  config = lib.mkIf cfg.daemonExperimental.enable {
    # DEPRECATED v1.2: kept as migration tombstone for the
    # nixling-launcher{,s} → nixling rename. No module references the
    # legacy groups; no user is a member. The empty declaration
    # preserves the legacy gid in /etc/group so the
    # nixlingGroupMigration helper can match by numeric gid on direct
    # upgrades. Slated for removal in v1.3 after one release of
    # confirmed clean migration.
    users.groups.nixling-launchers = { };
    users.groups.nixlingd = { };

    users.users =
      (lib.genAttrs cfg.site.launcherUsers (_: {
        extraGroups = [ "nixling" ];
      }))
      // {
        nixlingd = {
          isSystemUser = true;
          group = "nixlingd";
          description = "nixling daemon user";
          # nixlingd MUST be a supplementary member of nixling so it
          # can `chown(path,
          # None, Some(gid))` the public socket to the launcher
          # group on bind. Without this membership, the chown(2)
          # call returns EPERM (kernel allows chown-to-gid only
          # for one of the caller's groups, real/effective/
          # supplementary). The daemon failed at startup with
          # "internal-io" when chown(public.sock, -1, 1000)
          # returned EPERM.
          extraGroups = [ "nixling" ];
        };
      };

    environment.systemPackages = [ nixlingdPackage nixlingCliPackage nixlingCliShellArtifactsPackage nixlingActivationHelperPackage ];

    environment.etc."nixling/daemon-config.json" = {
      source = daemonConfigJson;
      mode = "0640";
      user = "root";
      group = "nixlingd";
    };

    systemd.tmpfiles.rules = [
      # nixlingd runs non-root, so it must own
      # /run/nixling, /run/nixling/locks, /run/nixling/state, and the
      # daemon.lock file. /etc/nixling and /var/lib/nixling stay
      # root-owned and group-readable by nixlingd (the broker
      # audit log under /var/lib/nixling/audit/ is broker-owned and
      # written by root; the daemon only reads). /etc/nixling/
      # config + bundle/host/processes are root:nixlingd 0640 so the
      # daemon reads without write.
      #
      # /run/nixling is group-owned by
      # `nixling` (mode 0750) so launcher users — members
      # of `nixling` via the daemon-config.json
      # `publicSocketGroup` — can `x` (traverse) the directory to
      # reach `public.sock`. With nixlingd:nixling 0750
      # owner nixlingd has rwx (read/write the dir; bind/remove the
      # socket); group nixling has r-x (list contents +
      # traverse to the socket file); world has --- (no access). The
      # public socket itself is mode 0660 group nixling
      # (see packages/nixlingd/src/lib.rs::bind_public_socket).
      "d /run/nixling 0750 nixlingd nixling -"
      "f /run/nixling/daemon.lock 0640 nixlingd nixlingd -"
      # /run/nixling/locks holds per-VM `flock(LOCK_EX |
      # LOCK_NB)` files taken by the daemon for the entire `up` /
      # `prepare` / `destroy` mutation window. Mode 0700 nixlingd
      # nixlingd so only the daemon (and root) can open the lock file.
      # Cleared on every boot via the standard
      # tmpfiles `d` rule semantics.
      "d /run/nixling/locks 0700 nixlingd nixlingd -"
      "d /run/nixling/state 0700 nixlingd nixlingd -"
      "d /var/lib/nixling 0750 root nixlingd -"
      "d /etc/nixling 0750 root nixlingd -"
    ];

    systemd.services.nixlingd = {
      # restartIfChanged = false is
      # required at the TOP LEVEL of systemd.services.<name> — NOT inside
      # serviceConfig or unitConfig. NixOS's switch-to-configuration only
      # reads the top-level NixOS option; the unitConfig.X-RestartIfChanged
      # form emits under [Unit] where switch-to-configuration silently
      # ignores it (same bug that was fixed for per-VM sidecars in v0.1.7).
      #
      # Why: nixlingd is the long-lived supervisor whose pidfd handle owns
      # the child runner DAG. A rebuild-triggered restart would atomically
      # tear down all in-flight VM processes — identical blast radius to the
      # per-VM sidecar restartIfChanged bug. The VM lifecycle policy
      # (AGENTS.md "VM lifecycle policy") extends to the daemon itself.
      # Operators apply daemon updates explicitly via `nixling daemon restart`
      # or a manual `systemctl restart nixlingd` after verifying quiescence.
      restartIfChanged = false;
      description = "nixling daemon skeleton";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];
      # Bypass the kernel-module fatal check because this host's kernel
      # (linux-7.0.5) has the guest-side
      # virtio modules (virtio_console, virtio_net, virtio_fs,
      # drm_virtio_gpu) built-in (=y) rather than loadable (=m),
      # which the daemon's lsmod-based check mis-reads as "missing".
      # See packages/nixlingd/src/lib.rs for the operator-override
      # env var.
      environment.NIXLING_SKIP_KERNEL_MODULE_CHECK = "1";
      serviceConfig = {
        Type = "simple";
        # cgroup v2 delegation requires the long-lived daemon to be
        # non-root so the broker can fchown
        # the nixling.slice subtree to the daemon's uid/gid. Running
        # the daemon as root contradicts the delegation contract in
        # docs/reference/cgroup-delegation.md and ADR 0011 ("the
        # daemon is never uid 0; the broker delegates the cgroup
        # subtree to the daemon user").
        User = "nixlingd";
        Group = "nixlingd";
        ExecStart = "${nixlingdPackage}/bin/nixlingd serve --config /etc/nixling/daemon-config.json";
        Restart = "on-failure";
        RestartSec = "2s";
        NoNewPrivileges = true;
        CapabilityBoundingSet = [ "" ];
        AmbientCapabilities = [ "" ];
        PrivateTmp = true;
        ProtectHome = true;
        # AF_UNIX: public.sock + broker IPC + the per-VM guest-control
        # vsock proxy socket the daemon-side authenticated Health probe
        # connects to. AF_INET/AF_INET6 remain for the daemon's other TCP
        # readiness probes (e.g. per-env usbipd backend ports).
        RestrictAddressFamilies = [ "AF_UNIX" "AF_INET" "AF_INET6" ];
        UMask = "0027";
        # Supplementary group so the daemon can create
        # /run/nixling/public.sock with group ownership
        # nixling (the documented launcher discovery group).
        # The matching publicSocketGroup field in
        # daemon-config.json already declares nixling as
        # the public socket group; this SupplementaryGroups entry
        # gives the systemd unit's primary uid the second gid it
        # needs to chgrp the socket.
        SupplementaryGroups = [ "nixling" ];
      };
    };
  };
}
