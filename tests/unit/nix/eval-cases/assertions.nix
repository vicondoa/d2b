# Consolidated case table for the legacy
# `tests/assertions-eval.sh` gate. ONE `nix-instantiate --eval --strict
# --json` of this file returns the per-case `failingMessages` /
# `evalSucceeded` map; the shell wrapper then asserts each case's
# expected substring against either the failing-assertion message list
# (Bucket A, the common path) or the captured throw signal
# (Bucket B, eval-throws — fallback to a focused per-case re-eval).
#
# Replaces 31 separate per-case `nix-instantiate --eval --strict`
# invocations in the legacy bash gate. See `shared.nix` for the
# evaluator contract.
{ flakeRoot ? null, nixpkgs ? null, d2bModule ? null }:

let
  shared = import ./shared.nix { inherit flakeRoot nixpkgs d2bModule; };
in
shared.mkBatch {
  cases = {
    # H10/1 — private-key marker in userAuthorizedKeys must be rejected.
    "private-key-in-authorized-keys" = {
      expectedSubstring = "does not look like a valid SSH public key";
      override = (
        { ... }:
        {
          d2b.site.userAuthorizedKeys = [
            "-----BEGIN OPENSSH PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEILa...\n-----END OPENSSH PRIVATE KEY-----"
          ];
        }
      );
    };

    # H10/2 — graphics VM declared but waylandUser = null.
    "graphics-without-wayland-user" = {
      expectedSubstring = "d2b.site.waylandUser";
      override = (
        { ... }:
        {
          d2b.site.waylandUser = null;
          d2b.vms.corp-vm.graphics.enable = true;
        }
      );
    };

    # H10/3 — waylandUser names a user that does not exist.
    "wayland-user-missing" = {
      expectedSubstring = "config.users.users.ghost is not declared";
      override = (
        { lib, ... }:
        {
          d2b.site.waylandUser = lib.mkForce "ghost";
        }
      );
    };

    # Naming surface — VM names must start with a letter and only
    # use lowercase alnum + '-'.
    "vm-name-invalid" = {
      expectedSubstring = "VM name must match the regex ^[a-z][a-z0-9-]*$";
      override = (
        { ... }:
        {
          d2b.vms = {
            "42web" = {
              enable = true;
              env = "work";
              index = 11;
              ssh.user = "alice";
              config = {
                networking.hostName = "42web";
                users.users.alice = {
                  isNormalUser = true;
                  uid = 1000;
                };
              };
            };
          };
        }
      );
    };

    # Naming surface — 'launcher' is reserved.
    "vm-name-reserved-launcher" = {
      expectedSubstring = "'launcher' is reserved";
      override = (
        { ... }:
        {
          d2b.vms = {
            launcher = {
              enable = true;
              env = "work";
              index = 11;
              ssh.user = "alice";
              config = {
                networking.hostName = "launcher";
                users.users.alice = {
                  isNormalUser = true;
                  uid = 1000;
                };
              };
            };
          };
        }
      );
    };

    # Naming surface — user-declared VMs may not consume sys-* prefix.
    "vm-name-reserved-sys-prefix" = {
      expectedSubstring = "names starting with 'sys-' are reserved";
      override = (
        { ... }:
        {
          d2b.vms = {
            "sys-shadow" = {
              enable = true;
              env = "work";
              index = 11;
              ssh.user = "alice";
              config = {
                networking.hostName = "sys-shadow";
                users.users.alice = {
                  isNormalUser = true;
                  uid = 1000;
                };
              };
            };
          };
        }
      );
    };

    # Naming surface — env names share the same leading-letter rule.
    "env-name-invalid" = {
      expectedSubstring = "env name must match the regex ^[a-z][a-z0-9-]*$";
      override = (
        { lib, ... }:
        {
          d2b.envs = {
            "9corp" = {
              lanSubnet = "10.99.0.0/24";
              uplinkSubnet = "198.51.100.0/30";
            };
          };
          d2b.vms.corp-vm.env = lib.mkForce "9corp";
        }
      );
    };

    # Network option-schema — env names must fit the IFNAMSIZ budget.
    "env-name-too-long" = {
      expectedSubstring = "env name must be at most 8 characters";
      override = (
        { lib, ... }:
        {
          d2b.envs = {
            corpwest1 = {
              lanSubnet = "10.99.0.0/24";
              uplinkSubnet = "198.51.100.0/30";
            };
          };
          d2b.vms.corp-vm.env = lib.mkForce "corpwest1";
        }
      );
    };

    # Network option-schema — workload env references must point at
    # a declared env.
    "vm-env-missing" = {
      expectedSubstring = "but d2b.envs has no such ENABLED env";
      override = (
        { lib, ... }:
        {
          d2b.vms.corp-vm.env = lib.mkForce "ghost";
        }
      );
    };

    # Network option-schema — workload env references may not target
    # a disabled env.
    "vm-env-disabled" = {
      expectedSubstring = "but d2b.envs has no such ENABLED env";
      override = (
        { lib, ... }:
        {
          d2b.envs.work.enable = lib.mkForce false;
          d2b.vms.corp-vm.env = lib.mkForce "work";
        }
      );
    };

    # Network option-schema — workload indices must be unique within
    # an env.
    "vm-index-duplicate" = {
      expectedSubstring = "Each workload VM in an env needs a unique `index`";
      override = (
        { ... }:
        {
          d2b.vms.other-vm = {
            enable = true;
            env = "work";
            index = 10;
            ssh.user = "alice";
            config = {
              networking.hostName = "other-vm";
              users.users.alice = {
                isNormalUser = true;
                uid = 1000;
              };
            };
          };
        }
      );
    };

    # Network option-schema — staticIp and env/index wiring are
    # mutually exclusive.
    "static-ip-and-env-mutually-exclusive" = {
      expectedSubstring = "set EITHER `env`/`index` OR the deprecated `staticIp`, not both";
      override = (
        { lib, ... }:
        {
          d2b.vms.corp-vm.staticIp = lib.mkForce "10.20.0.50";
        }
      );
    };

    # H10/4 — lanSubnet must be /24.
    "lansubnet-wrong-mask" = {
      expectedSubstring = "must be a /24";
      override = (
        { lib, ... }:
        {
          d2b.envs.work.lanSubnet = lib.mkForce "10.99.0.0/23";
        }
      );
    };

    # H10/5 — uplinkSubnet must be /30.
    "uplinksubnet-wrong-mask" = {
      expectedSubstring = "must be a /30";
      override = (
        { lib, ... }:
        {
          d2b.envs.work.uplinkSubnet = lib.mkForce "192.0.2.0/29";
        }
      );
    };

    # H10/6 — lanSubnet network address must end in .0.
    "lansubnet-nonzero-host" = {
      expectedSubstring = "ending in '.0'";
      override = (
        { lib, ... }:
        {
          d2b.envs.work.lanSubnet = lib.mkForce "10.99.0.5/24";
        }
      );
    };

    # H10/7 — two envs whose CIDRs OVERLAP.
    "overlap-containment" = {
      expectedSubstring = "CIDR overlap";
      override = (
        { ... }:
        {
          d2b.envs.other = {
            lanSubnet = "10.20.0.0/16";
            uplinkSubnet = "198.51.100.0/30";
          };
        }
      );
    };

    # H10/8 — env subnet overlaps with a hostLanCidrs entry.
    "env-vs-host-overlap" = {
      expectedSubstring = "overlaps with `d2b.hostLanCidrs`";
      override = (
        { ... }:
        {
          d2b.hostLanCidrs = [ "10.20.0.0/16" ];
        }
      );
    };

    # Wave 3 — stateDir is reserved but not fully threaded.
    "state-dir-override-rejected" = {
      expectedSubstring = "d2b.site.stateDir is reserved but not fully threaded yet";
      override = (
        { lib, ... }:
        {
          d2b.site.stateDir = lib.mkForce "/persist/d2b";
        }
      );
    };

    "store-state-dir-override-rejected" = {
      expectedSubstring = "d2b.store.stateDir is reserved but not fully threaded yet";
      override = (
        { lib, ... }:
        {
          d2b.store.stateDir = lib.mkForce "/persist/d2b/vms";
        }
      );
    };

    "allow-east-west-requires-site-ack" = {
      expectedSubstring = "allowUnsafeEastWest = true";
      override = (
        { ... }:
        {
          d2b.envs.work.lan.allowEastWest = true;
        }
      );
    };

    "home-lan-attachment-requires-host-interface" = {
      expectedSubstring = "homeLan.attachment.enable requires";
      override = (
        { ... }:
        {
          d2b.envs.work.homeLan.attachment.enable = true;
        }
      );
    };

    "home-lan-egress-requires-attachment" = {
      expectedSubstring = "homeLan.egress.enable requires";
      override = (
        { ... }:
        {
          d2b.envs.work.homeLan.egress.enable = true;
        }
      );
    };

    "home-lan-port-forward-requires-attachment" = {
      expectedSubstring = "homeLan.portForwards requires";
      override = (
        { ... }:
        {
          d2b.envs.work.homeLan.portForwards = [{
            protocol = "tcp";
            listenPort = 8443;
            vm = "corp-vm";
            targetPort = 443;
          }];
        }
      );
    };

    "home-lan-mdns-requires-attachment" = {
      expectedSubstring = "homeLan.mdns.enable requires";
      override = (
        { ... }:
        {
          d2b.envs.work.homeLan.mdns.enable = true;
        }
      );
    };

    "home-lan-port-forward-required-fields" = {
      expectedSubstring = "must specify either vm or targetIp";
      override = (
        { ... }:
        {
          d2b.envs.work.homeLan.attachment = {
            enable = true;
            interface = "eno1";
          };
          d2b.envs.work.homeLan.portForwards = [{
            listenPort = 8443;
          }];
        }
      );
    };

    "home-lan-port-forward-target-same-env" = {
      expectedSubstring = "must name an enabled VM in the same env";
      override = (
        { ... }:
        {
          d2b.envs.other = {
            lanSubnet = "10.30.0.0/24";
            uplinkSubnet = "198.51.100.0/30";
          };
          d2b.vms.other-vm = {
            enable = true;
            env = "other";
            index = 10;
            ssh.user = "alice";
          };
          d2b.envs.work.homeLan.attachment = {
            enable = true;
            interface = "eno1";
          };
          d2b.envs.work.homeLan.portForwards = [{
            protocol = "tcp";
            listenPort = 8443;
            vm = "other-vm";
            targetPort = 443;
          }];
        }
      );
    };

    "home-lan-egress-peer-cidr-rejected" = {
      expectedSubstring = "homeLan.egress.allowedCidrs entry";
      override = (
        { ... }:
        {
          d2b.envs.other = {
            lanSubnet = "10.30.0.0/24";
            uplinkSubnet = "198.51.100.0/30";
          };
          d2b.envs.work.homeLan.attachment = {
            enable = true;
            interface = "eno1";
          };
          d2b.envs.work.homeLan.egress = {
            enable = true;
            allowedCidrs = [ "10.30.0.0/24" ];
          };
        }
      );
    };

    "home-lan-port-forward-source-peer-cidr-rejected" = {
      expectedSubstring = "homeLan.portForwards[0].sourceCidrs";
      override = (
        { ... }:
        {
          d2b.envs.other = {
            lanSubnet = "10.30.0.0/24";
            uplinkSubnet = "198.51.100.0/30";
          };
          d2b.envs.work.homeLan.attachment = {
            enable = true;
            interface = "eno1";
          };
          d2b.envs.work.homeLan.portForwards = [{
            protocol = "tcp";
            listenPort = 8443;
            vm = "corp-vm";
            targetPort = 443;
            sourceCidrs = [ "10.30.0.0/24" ];
          }];
        }
      );
    };

    # graphics.enable on aarch64-linux must trip the
    # host.nix platform gate.
    "platform-gate-graphics-aarch64" = {
      expectedSubstring = "graphics/audio components are";
      system = "aarch64-linux";
      override = (
        { ... }:
        {
          d2b.vms.corp-vm.graphics.enable = true;
        }
      );
    };

    # audio.enable on aarch64-linux must also trip the gate.
    "platform-gate-audio-aarch64" = {
      expectedSubstring = "graphics/audio components are";
      system = "aarch64-linux";
      override = (
        { ... }:
        {
          d2b.vms.corp-vm.audio.enable = true;
        }
      );
    };

    # v0.1.6 SWArch-M9 — graphics VMs cannot be autostart.
    "graphics-with-autostart" = {
      expectedSubstring = "graphics.enable = true is incompatible";
      override = (
        { ... }:
        {
          d2b.vms.corp-vm.graphics.enable = true;
          d2b.vms.corp-vm.autostart = true;
        }
      );
    };

    # graphics.xwayland.enable = true fails closed during Wayland-only migration.
    "graphics-xwayland-unsupported" = {
      expectedSubstring = "supported in this release";
      override = (
        { ... }:
        {
          d2b.vms.corp-vm.graphics.enable = true;
          d2b.vms.corp-vm.graphics.xwayland.enable = true;
        }
      );
    };

    # Issue #22 — guest audit forwarding requires per-VM observability.
    "audit-without-observability" = {
      expectedSubstring = "d2b.vms.corp-vm.audit.enable requires observability.enable on the same VM";
      override = (
        { ... }:
        {
          d2b.vms.corp-vm.audit.enable = true;
        }
      );
    };

    # W19 — `guest.exec.allowRoot` was removed (exec always runs as the
    # workload user). A legacy assignment must land on the friendly
    # migration assertion, not a cryptic "option does not exist".
    "guest-exec-allowroot-removed" = {
      expectedSubstring = "guest.exec.allowRoot was removed";
      override = (
        { ... }:
        {
          d2b.vms.corp-vm.guest.exec.allowRoot = true;
        }
      );
    };

    # W19 — `guest.exec.users` was removed (no per-VM exec user
    # allowlist; exec targets the single workload user `ssh.user`).
    "guest-exec-users-removed" = {
      expectedSubstring = "guest.exec.users was removed";
      override = (
        { ... }:
        {
          d2b.vms.corp-vm.guest.exec.users = [ "alice" ];
        }
      );
    };

    # v1.1.2fu19 panel-test R2 must-fix: stablePrincipalId UID
    # collision assertion (per the new check in
    # nixos-modules/minijail-profiles.nix:538-575). vm2672 and
    # vm8350 are a known-colliding pair whose
    # `d2b-<name>-runner` SHA-256 prefixes both map to UID
    # 2442195 = 50000 + 0x248083. Two enabled VMs with these
    # names MUST trigger the assertion at eval time. The expected
    # substring is from the assertion message template.
    "principal-uid-collision" = {
      expectedSubstring = "v1.1.2 stablePrincipalId collision: UID 2442195";
      override = (
        { lib, ... }:
        {
          d2b.vms.vm2672 = {
            enable = true;
            env = "work";
            index = 30;
            ssh.user = "alice";
          };
          d2b.vms.vm8350 = {
            enable = true;
            env = "work";
            index = 31;
            ssh.user = "alice";
          };
        }
      );
    };

    # The former "observability-reserved-cid" negative case was removed:
    # it is unsatisfiable under the current vsock CID formula. Workload
    # CIDs are `100 + envIndex*1000 + slot` (nixos-modules/lib.nix
    # `guestControlVsockCid`) with `index` typed `ints.between 10 250`,
    # so every workload VM lands in [110+envIndex*1000, 350+envIndex*1000].
    # The reserved observability CID (1000) sits in a permanent gap no
    # type-valid workload VM can reach, so the `Vsock CID 1000 is reserved`
    # assertion (nixos-modules/assertions.nix) is defense-in-depth that
    # cannot be triggered by a valid config. Verified: the old case config
    # produced corp-vm=1300 with sys-obs=1000 (the obs VM itself, which is
    # excluded from the collision set).
  };
}
