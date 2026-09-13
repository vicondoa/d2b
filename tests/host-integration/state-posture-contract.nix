# Type-G runNixOSTest: declared host posture contract for state farms and
# shared directories (issue #512).
#
# One file declares the posture (`packages/d2b-broker/src/ops/
# state-posture-contract.json`); the broker posture code embeds it, the Nix
# provisioning derives from it, and this fixture asserts the live host against
# it: every declared level's owner/group/mode/ACL, plus the allowed AND denied
# operations for each principal (root, d2bd, a d2b-group launcher, and nobody).
# The fixture boots the same Zone-native Cloud Hypervisor Guest recipe the
# acceptance fixture uses, so the state chain, the store-view farm, and the
# spawn-time traversal ACLs are all materialized by the product path.
{ pkgs, self }:

let
  inherit (pkgs) lib;
  d2bLib = import ./lib.nix {
    inherit self;
    inherit lib;
    hostToolBundle =
      if self.lib ? d2bHostToolBundle then self.lib.d2bHostToolBundle else null;
  };
  cloudHypervisorArtifact =
    d2bLib.mkRuntimeCloudHypervisorArtifact pkgs;
  volumeProviderArtifact = d2bLib.mkVolumeProviderArtifact pkgs;
  fixtureKeys = pkgs.runCommand "acceptance-component-session-keys" { } ''
    mkdir -p "$out"
    printf '\001\002\003\004\005\006\007\010\011\012\013\014\015\016\017\020\021\022\023\024\025\026\027\030\031\032\033\034\035\036\037\040' > "$out/host.key"
    printf '\007\243\174\274\024\040\223\310\267\125\334\033\020\350\154\264\046\067\112\321\152\250\123\355\013\337\300\262\270\155\034\174' > "$out/host.pub"
    printf '\041\042\043\044\045\046\047\050\051\052\053\054\055\056\057\060\061\062\063\064\065\066\067\070\071\072\073\074\075\076\077\100' > "$out/guest.key"
    printf '\130\151\257\364\120\124\227\062\313\252\355\136\135\371\263\012\155\243\034\260\345\164\053\255\132\324\241\247\150\361\246\173' > "$out/guest.pub"
  '';
  guestBundle = pkgs.runCommand "acceptance-guest-bundle" {
    nativeBuildInputs = [ pkgs.python3 ];
  } ''
    mkdir -p "$out"
    cat > "$out/host.json" <<'EOF'
    {"schemaVersion":"v2","site":{"allowUnsafeEastWest":false},"environments":[],"nftables":{"family":"inet","table":"d2b","chains":[],"tableHashAfterApply":null,"ownershipId":"host-integration"},"networkManager":{"filePath":"/etc/NetworkManager/conf.d/00-d2b-unmanaged.conf","matchCriteria":[],"reloadBehavior":"atomic-reload","ownership":{"owner":"root","group":"root","mode":"0644","driftPolicy":"replace"}},"hostsFile":{"startMarker":"# d2b-managed begin","endMarker":"# d2b-managed end","rule":"replace-managed-block"},"kernelModules":[],"fdOwnership":[],"cloudHypervisorCapabilities":[],"ifNameMappings":[],"ch":null,"firewallCoexistencePolicy":null}
    EOF
    printf '%s\n' '{"schemaVersion":"v2","vms":[]}' > "$out/processes.json"
    printf '%s\n' '{"schemaVersion":"v2","publicOperations":[],"brokerOperations":[]}' > "$out/privileges.json"
    printf '%s\n' '{"_manifest":{"manifestVersion":6},"_observability":{"enabled":false,"signozUrl":"http://127.0.0.1:8080","signozOtlpGrpcPort":4317,"signozOtlpHttpPort":4318,"obsVsockCid":0,"obsVsockHostSocket":"","vmName":""}}' > "$out/vms.json"
    python3 - "$out/bundle.json" <<'PY'
    import hashlib
    import json
    import sys

    bundle = {
        "artifactHashes": None,
        "bundleVersion": 4,
        "closures": [],
        "generation": {
            "generatedAt": None,
            "generator": "host-integration",
            "sourceRevision": None,
        },
        "hostPath": "host.json",
        "managedKeys": {
            "keysDir": "/var/lib/d2b/keys",
            "knownHostsPath": "/var/lib/d2b/known_hosts.d2b",
            "overrides": [],
        },
        "minijailProfiles": [],
        "privilegesPath": "privileges.json",
        "processesPath": "processes.json",
        "publicManifestPath": "vms.json",
        "schemaVersion": "v2",
    }
    canonical = json.dumps(bundle, sort_keys=True, separators=(",", ":")).encode()
    bundle["bundleHash"] = "sha256:" + hashlib.sha256(canonical).hexdigest()
    with open(sys.argv[1], "w", encoding="utf-8") as output:
        json.dump(bundle, output, sort_keys=True, separators=(",", ":"))
        output.write("\n")
    PY
  '';

  cloudHypervisorConfig = {
    controllerExecutionRef = "Host/host-system";
    defaultVcpus = 2;
    defaultMemoryMb = 512;
    defaultMachineType = "microvm";
    watchdog = true;
    adoptionWindowMs = 30000;
    healthCheckIntervalMs = 5000;
    healthCheckTimeoutMs = 1000;
    healthCheckFailureThreshold = 3;
    startupDeadlineMs = 120000;
  };
  guestSystem = d2bLib.mkGuestSystem {
    inherit pkgs;
    name = "acceptance-guest";
    modules = [
      ({ lib, ... }: {
        boot.kernelParams = [ "console=ttyS0" "loglevel=7" ];
        environment.etc."d2b/component-session/guest.key".source =
          "${fixtureKeys}/guest.key";
        environment.etc."d2b/component-session/parent.pub".source =
          "${fixtureKeys}/host.pub";
        systemd.services.d2bd-guest = {
          environment = {
            RUST_LOG = "d2bd=debug";
          };
          serviceConfig = {
            ReadOnlyPaths = [
              "/etc/d2b/component-session/guest.key"
              "/etc/d2b/component-session/parent.pub"
            ];
            StandardOutput = lib.mkForce "journal+console";
            StandardError = lib.mkForce "journal+console";
          };
        };
        systemd.services.d2b-test-boot-identity = {
          wantedBy = [ "basic.target" ];
          before = [ "d2bd-guest.service" ];
          serviceConfig.Type = "oneshot";
          script = ''
            printf 'D2B_GUEST_BOOT_ID=%s\n' \
              "$(${pkgs.coreutils}/bin/cat /proc/sys/kernel/random/boot_id)" \
              > /dev/console
          '';
        };
        d2b.componentSession.localPrivateKeyPath =
          "/etc/d2b/component-session/guest.key";
        d2b.componentSession.parentPublicKeyPath =
          "/etc/d2b/component-session/parent.pub";
        d2b.componentSession.bundlePath =
          "/var/lib/d2b/guest-bundle/bundle.json";
        d2b.guestBroker.bundlePath =
          "/var/lib/d2b/guest-bundle/bundle.json";
        systemd.services.d2b-install-guest-bundle = {
          requiredBy = [ "d2b-broker-guest.service" "d2bd-guest.service" ];
          before = [ "d2b-broker-guest.service" "d2bd-guest.service" ];
          serviceConfig.Type = "oneshot";
          script = ''
            install -d -o root -g d2bd -m 0750 /var/lib/d2b/guest-bundle
            for file in bundle.json host.json processes.json privileges.json; do
              install -o root -g d2bd -m 0640 \
                ${guestBundle}/"$file" /var/lib/d2b/guest-bundle/"$file"
            done
            install -o root -g d2bd -m 0644 \
              ${guestBundle}/vms.json /var/lib/d2b/guest-bundle/vms.json
          '';
        };
        networking.useDHCP = lib.mkForce false;
        networking.networkmanager.enable = lib.mkForce false;
        systemd.network.enable = lib.mkForce false;
        services.dbus.enable = lib.mkForce false;
        services.resolved.enable = lib.mkForce false;
        systemd.services.systemd-vconsole-setup.enable = false;
        microvm.storeOnDisk = true;
        microvm.storeDisk = guestStoreDisk;
        microvm.shares = lib.mkForce [ ];
        fileSystems."/nix/store" = {
          device = "/dev/vda";
          fsType = "ext4";
          options = [ "ro" "x-initrd.mount" ];
          neededForBoot = true;
        };
      })
    ];
  };
  guestClosure = pkgs.closureInfo {
    rootPaths = [ guestSystem.config.system.build.toplevel ];
  };
  guestStoreDisk = pkgs.runCommand "acceptance-guest-store.img" {
    nativeBuildInputs = [ pkgs.coreutils pkgs.e2fsprogs ];
  } ''
    mkdir -p root
    while IFS= read -r path; do
      cp -r --no-preserve=ownership,xattr,context "$path" root/
    done < ${guestClosure}/store-paths
    truncate -s 4096M "$out"
    mkfs.ext4 -q -F -d root "$out"
  '';
  artifacts = {
    runtime-cloud-hypervisor = {
      inherit (cloudHypervisorArtifact) package type catalog;
    };
    volume-acceptance-provider = {
      inherit (volumeProviderArtifact) package type catalog;
    };
    acceptance-system = {
      package = guestSystem.config.system.build.toplevel;
      type = "nixos-system";
    };
  };
in
pkgs.testers.runNixOSTest {
  name = "d2b-state-posture-contract";

  nodes.machine = d2bLib.d2bCloudHypervisorNode {
    extra = { ... }: {
      d2b.site.adminUsers = [ "alice" ];
      environment.systemPackages = with pkgs; [
        acl
        iproute2
        jq
        iputils
        procps
        util-linux
      ];
      d2b.artifacts = artifacts;
      # The declaration under test, installed verbatim from the repo so the
      # fixture reads the same file the posture code embeds.
      environment.etc."d2b/state-posture-contract.json".source =
        ../../packages/d2b-broker/src/ops/state-posture-contract.json;
      d2b.guestSystems.work.acceptance-guest = guestSystem;
      d2b.zones.local-root.trustedPublishers.d2b-cloud-hypervisor.signingKey =
        cloudHypervisorArtifact.trustedPublisher.signingKey;
      d2b.zones.local-root.trustedPublishers.d2b-volume-acceptance.signingKey =
        volumeProviderArtifact.trustedPublisher.signingKey;
      d2b.zones.work.trustedPublishers.d2b-cloud-hypervisor.signingKey =
        cloudHypervisorArtifact.trustedPublisher.signingKey;
      d2b.zones.work.trustedPublishers.d2b-volume-acceptance.signingKey =
        volumeProviderArtifact.trustedPublisher.signingKey;
      d2b.zones.local-root.resources.host-system = {
        type = "Host";
        spec = {
          providerRef = "Provider/system-core";
          defaultDomain = "system";
          allowedDomains = [ "system" ];
          budget = { };
          networkAttachments = [ ];
          deviceAttachments = [ ];
          volumeAttachmentDefaults = [ ];
        };
      };
      d2b.zones.work = {
        parentZone = "local-root";
        resources = {
          alice = {
            type = "User";
            spec = {
              displayName = "Alice";
              groups = [ ];
              osUsername = "alice";
            };
          };
          d2bd = {
            type = "User";
            spec = {
              displayName = "d2bd";
              groups = [ ];
              osUsername = "d2bd";
            };
          };
          lifecycle-operator = {
            type = "Role";
            spec.rules = [
              {
                resourceTypes = [ "Endpoint" "Guest" "Host" "Process" "Provider" "Volume" "VolumeBinding" ];
                verbs = [ "get" "list" ];
                subresources = [ ];
                resourceNames = [ ];
                zones = [ "work" ];
                executionRefs = [ ];
                sessionVerbs = [ "connect" "invoke" ];
              }
              {
                resourceTypes = [ "Guest" ];
                verbs = [ "delete" ];
                subresources = [ ];
                resourceNames = [ "acceptance-guest" ];
                zones = [ "work" ];
                executionRefs = [ ];
                sessionVerbs = [ "connect" "invoke" ];
              }
              {
                resourceTypes = [ "Volume" ];
                verbs = [ "delete" ];
                subresources = [ ];
                resourceNames = [ "state" ];
                zones = [ "work" ];
                executionRefs = [ ];
                sessionVerbs = [ "connect" "invoke" ];
              }
            ];
          };
          lifecycle-operator-binding = {
            type = "RoleBinding";
            spec = {
              roleRef = "Role/lifecycle-operator";
              subjects = [ "User/alice" ];
              externalPrincipalSelector = null;
              scopeNarrowing = null;
            };
          };
          host-system = {
            type = "Host";
            spec = {
              providerRef = "Provider/system-core";
              defaultDomain = "system";
              allowedDomains = [ "system" ];
              budget = { };
              networkAttachments = [ ];
              deviceAttachments = [ ];
              volumeAttachmentDefaults = [ ];
            };
          };
          volume-local = {
            type = "Provider";
            spec = {
              artifactId = "volume-acceptance-provider";
              config = {
                controllerExecutionRef = "Host/host-system";
                sourcePolicies = [
                  {
                    id = "default-state";
                    class = "local-path";
                    volumeKinds = [ "durable" "state" "cache" ];
                  }
                  # U7: daemon-owned root the unprivileged daemon can
                  # lock and provision inline (path:daemon-state).
                  {
                    id = "daemon-state";
                    class = "local-path";
                    volumeKinds = [ "durable" "state" "cache" ];
                  }
                ];
              };
            };
          };
          volume-virtiofs = {
            type = "Provider";
            spec = {
              artifactId = "volume-acceptance-provider";
              config.controllerExecutionRef = "Host/host-system";
            };
          };
          state = {
            type = "Volume";
            spec = {
              providerRef = "Provider/volume-local";
              kind = "state";
              source = {
                executionRef = "Host/host-system";
                settings = {
                  kind = "local-path";
                  sourcePolicyId = "daemon-state";
                };
              };
              layout = [{
                path = "state";
                type = "directory";
                # U7: daemon-owned so the unprivileged daemon can
                # provision inline; the guest share stays read-only.
                ownerRef = "User/d2bd";
                groupRef = "User/d2bd";
                mode = "0700";
                target = null;
                accessAcl = [ ];
                defaultAcl = [ ];
                foreignChildPolicy = "preserve";
                noFollow = true;
                recursive = false;
                sensitivity = "private";
                createPolicy = "create-if-never-provisioned";
                repairPolicy = "exact-owner";
                cleanupPolicy = "owner-controlled";
                adoptionPolicy = "quarantine-on-ambiguity";
                restartPolicy = "preserve-across-controller-restart";
                leaseClass = "none";
                invariants = [ "no-symlink" ];
              }];
              views.controller = {
                path = "";
                rights = [ "read" "write" "traverse" ];
              };
              # KTD1: the attachment stays declared input only. The Volume
              # side mints the durable VolumeBinding at reconcile; the
              # deterministic binding identity below is
              # vol-binding-6a8ea4307a30f7ceae6533f2 (volume, execution
              # target, view, mount path).
              attachments = [{
                executionRef = "Guest/acceptance-guest";
                transport = "virtiofs";
                view = "controller";
                access = "read-only";
                mountPath = "/state";
                settings = {
                  posixAcl = false;
                  xattr = false;
                  cache = "auto";
                  inodeFileHandles = "never";
                  threadPoolSize = null;
                  socketGroup = null;
                };
              }];
            };
          };
          runtime-cloud-hypervisor = {
            type = "Provider";
            spec = {
              artifactId = "runtime-cloud-hypervisor";
              config = cloudHypervisorConfig;
            };
          };
          acceptance-guest = {
            type = "Guest";
            spec = {
              providerRef = "Provider/runtime-cloud-hypervisor";
              executionRef = "Host/host-system";
              systemArtifactId = "acceptance-system";
              defaultDomain = "system";
              allowedDomains = [ "system" ];
              budget = { };
              volumeAttachmentDefaults = [ ];
              networkAttachments = [ ];
              deviceAttachments = [ ];
            };
          };
        };
      };
    };
  };

  testScript = ''
    ${d2bLib.fixtureDiagnostics}

    import json as _json
    import shlex as _shlex

    ZONE = "work"
    GUEST = "acceptance-guest"
    STATE_ROOT = "/var/lib/d2b"
    PRINCIPAL_USER = {
        "root": "root",
        "d2bd": "d2bd",
        "d2b": "alice",
        "nobody": "nobody",
    }

    def check(condition, message):
        if not condition:
            raise AssertionError(message)

    def tokens():
        return {
            "state-root": STATE_ROOT,
            "zone": ZONE,
            "guest": GUEST,
            "vm": GUEST,
        }

    def substitute(value):
        for token, replacement in tokens().items():
            value = value.replace("<" + token + ">", replacement)
        return value

    def tree(tree_id):
        matches = [entry for entry in CONTRACT["trees"] if entry["id"] == tree_id]
        check(len(matches) == 1, "contract tree " + tree_id + " must exist exactly once")
        return matches[0]

    def level_path(entry, level):
        root = substitute(entry["root"])
        if level["path"] == ".":
            return root
        return root + "/" + substitute(level["path"])

    def stat_row(path):
        output = machine.succeed("stat -c '%U %G %a' " + _shlex.quote(path))
        owner, group, mode = output.split()
        return owner, group, mode

    def acl_entries(path):
        output = machine.succeed(
            "getfacl -cp " + _shlex.quote(path) + " 2>/dev/null || true"
        )
        entries = set()
        for line in output.splitlines():
            parts = line.strip().split(":")
            if len(parts) != 3:
                continue
            kind, name, permissions = parts
            short = {"user": "u", "group": "g", "other": "o", "mask": "m"}.get(kind)
            if short is not None:
                entries.add(short + ":" + name + ":" + permissions)
        return entries

    def run_as(principal, command):
        uid, gid = PRINCIPAL_IDS[principal]
        status, _ = machine.execute(
            "setpriv --reuid="
            + uid
            + " --regid="
            + gid
            + " --init-groups /bin/sh -c "
            + _shlex.quote(command)
        )
        return status == 0

    def probe(principal, right, path):
        flag = {"traverse": "x", "read": "r", "write": "w"}[right]
        return run_as(principal, "test -" + flag + " " + _shlex.quote(path))

    def check_level(entry, level):
        path = level_path(entry, level)
        where = entry["id"] + ":" + level["path"] + " (" + path + ")"
        present = machine.execute("test -e " + _shlex.quote(path))[0] == 0
        if not present:
            check(
                not level.get("required", True),
                "declared level is missing: " + where,
            )
            return

        owner, group, mode_text = stat_row(path)
        mode = int(mode_text, 8)
        policy = level.get("modePolicy", "exact")
        if policy == "exact":
            check(
                mode == int(level["mode"], 8) & 0o7777,
                "mode drift at " + where + ": declared " + level["mode"]
                + ", observed " + mode_text,
            )
            check(
                owner == level["owner"],
                "owner drift at " + where + ": declared " + level["owner"]
                + ", observed " + owner,
            )
            check(
                group == level["group"],
                "group drift at " + where + ": declared " + level["group"]
                + ", observed " + group,
            )
        elif policy == "group-traverse-minimum":
            check(
                owner == level["owner"],
                "owner drift at " + where + ": declared " + level["owner"]
                + ", observed " + owner,
            )
            check(
                group == level["group"],
                "group drift at " + where + ": declared " + level["group"]
                + ", observed " + group,
            )
            check(mode & 0o010 != 0, "group search missing at " + where)
            check(mode & 0o020 == 0, "group write must never be granted at " + where)
        else:
            check(
                policy == "preserve-existing",
                "unknown modePolicy " + policy + " at " + where,
            )

        entries = acl_entries(path)
        for declared_acl in level.get("acl", []):
            check(
                declared_acl["spec"] in entries,
                "declared ACL missing at " + where + ": " + declared_acl["spec"],
            )

        for principal, rights in level["rights"].items():
            for right in ("traverse", "read", "write"):
                expectation = rights.get(right, "preserve")
                if expectation in ("preserve", "not-required"):
                    continue
                if principal == "root":
                    # root bypasses DAC; its allow rows are documentation.
                    continue
                observed = probe(principal, right, path)
                check(
                    observed == (expectation == "allow"),
                    where + ": " + principal + " " + right + " expected "
                    + expectation + ", observed "
                    + ("allow" if observed else "deny"),
                )

    start_all()
    stage("daemon-up")

    # The declared contract, from the same file the broker posture code embeds
    # and nixos-modules/host-daemon.nix derives its provisioning from. This
    # fixture never restates a posture value; it reads the declaration.
    CONTRACT = _json.loads(
        machine.succeed("cat /etc/d2b/state-posture-contract.json")
    )

    diag_unit("daemon-up", "d2bd.service", 180)
    machine.wait_for_unit("d2b-broker.socket", timeout=30)
    machine.wait_for_file("/run/d2b/public.sock", timeout=30)
    machine.succeed("systemctl start d2b-broker.service")
    diag_unit("broker-service", "d2b-broker.service", 30)

    PRINCIPAL_IDS = {
        principal: (
            machine.succeed("id -u " + user).strip(),
            machine.succeed("id -g " + user).strip(),
        )
        for principal, user in PRINCIPAL_USER.items()
    }

    guest_state = STATE_ROOT + "/zones/" + ZONE + "/guests/" + GUEST
    store_view = guest_state + "/store-view"

    stage("store-view-sync")
    diag_wait(
        "store-view-sync",
        "test -L " + store_view + "/state/current && test -L "
        + store_view + "/meta/current",
        300,
        rows=[
            (
                "guest state tree",
                "find " + guest_state + " -maxdepth 1 -exec stat -c '%A %U %G %n' {} + 2>/dev/null | sort || true",
            ),
            (
                "store-view tree",
                "find " + store_view + " -maxdepth 2 -exec stat -c '%A %U %G %n' {} + 2>/dev/null | sort || true",
            ),
        ],
        explain=[("d2bd.service", "store"), ("d2b-broker.service", "StoreSync")],
    )

    stage("vmm-spawn")
    diag_wait(
        "vmm-spawn",
        "test -S " + guest_state + "/" + GUEST + ".sock",
        300,
        rows=[
            (
                "guest state tree",
                "find " + guest_state + " -maxdepth 1 -exec stat -c '%A %U %G %n' {} + 2>/dev/null | sort || true",
            ),
        ],
        explain=[
            ("d2bd.service", "cloud-hypervisor"),
            ("d2bd.service", "component-session"),
        ],
    )

    stage("posture-contract")
    for tree_id in (
        "state-root",
        "guest-state-chain",
        "guest-state-dir",
        "guest-store-view",
        "shared-run-dir",
    ):
        entry = tree(tree_id)
        for level in entry["levels"]:
            check_level(entry, level)

    stage("anchor-open-rule")
    # The guest start above ran the daemon's anchored store-view walk while the
    # chain was capped at search-only by the spawn-time u:d2bd:--x ACL. Two
    # live proofs: the store-view open never failed, and the resolved view
    # reached a virtiofsd worker through the daemon's fd handoff.
    machine.fail(
        "journalctl -u d2bd.service --no-pager -b -n 5000 "
        "| grep -F 'store-view-open'"
    )
    check(
        machine.execute("pgrep -x virtiofsd >/dev/null")[0] == 0,
        "the store-view directory must reach a virtiofsd worker",
    )
    # Live denial, from the same contract rows: the daemon may search the
    # per-Guest state dir it does not own but may not read it.
    check(
        not run_as("d2bd", "ls " + _shlex.quote(guest_state) + " >/dev/null 2>&1"),
        "the daemon must not read the per-Guest state dir it only traverses",
    )

    stage("done")
    print("[d2b] declared state posture contract holds on the live host")
  '';
}
