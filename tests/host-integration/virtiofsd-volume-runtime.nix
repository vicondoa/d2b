# Type-G runNixOSTest: virtiofsd Volume slice with owned children on the v3
# resource runtime.
#
# This is the U11 midpoint Volume proof the plan names for the new (v3)
# resource plane, separate from the Process-slice operator-activation fixture:
# the Nix-ingested Volume derives one deterministic VolumeBinding child
# through the manager, the binding owns the virtiofsd worker Process and its
# private Endpoint, the worker is realized and binds its private serving
# socket, and deleting the Volume tears the chain down endpoint-first
# (R9/F3). All assertions read observable resource identity, phase,
# generation, processes, and sockets through the installed d2b CLI and the
# host filesystem; nothing keys on informational log text.
{ pkgs, self }:

let
  inherit (pkgs) lib;
  d2bLib = import ./lib.nix {
    inherit self;
    inherit lib;
    hostToolBundle =
      if self.lib ? d2bHostToolBundle then self.lib.d2bHostToolBundle else null;
  };
  volumeProviderArtifact = d2bLib.mkVolumeProviderArtifact pkgs;
  artifacts = {
    volume-acceptance-provider = {
      inherit (volumeProviderArtifact) package type catalog;
    };
  };
  hostRuntime = pkgs.writeText "d2b-acceptance-host-runtime.json" (builtins.toJSON {
    schemaVersion = "v1";
    bundleVersion = 1;
    generatedAt = "1970-01-01T00:00:00.000Z";
    nftAppliedHash = null;
    ifnames = [ ];
  });
in
pkgs.testers.runNixOSTest {
  name = "d2b-virtiofsd-volume-runtime";

  nodes.machine = d2bLib.d2bDaemonNode {
      extra = { ... }: {
        networking.nftables.enable = true;
        networking.nftables.ruleset = lib.mkAfter ''
          table inet d2b {}
        '';
        systemd.tmpfiles.rules = [
          "d /etc/NetworkManager/conf.d 0755 root root -"
        ];
        environment.etc."d2b/acceptance-host-runtime.json".source = hostRuntime;
        d2b.site.adminUsers = [ "alice" ];
        systemd.services.d2bd.serviceConfig.ExecStartPre = lib.mkAfter [
          "+${pkgs.writeShellScript "d2b-acceptance-hosts-prep" ''
            if [ -L /etc/hosts ]; then
              ${pkgs.coreutils}/bin/cat /etc/hosts > /run/d2b-acceptance-hosts
              ${pkgs.coreutils}/bin/rm -f /etc/hosts
              ${pkgs.coreutils}/bin/install -o root -g root -m 0644 \
                /run/d2b-acceptance-hosts /etc/hosts
            fi
          ''}"
          "+${pkgs.writeShellScript "d2b-acceptance-host-runtime-prep" ''
            ${pkgs.coreutils}/bin/install -D -o root -g d2bd -m 0640 \
              /etc/d2b/acceptance-host-runtime.json \
              /var/lib/d2b/runtime/host-runtime.json
          ''}"
        ];
        d2b.artifacts = artifacts;
        d2b.zones.local-root.trustedPublishers.d2b-volume-acceptance.signingKey =
          volumeProviderArtifact.trustedPublisher.signingKey;
      d2b.zones.work.parentZone = "local-root";
      d2b.zones.work.trustedPublishers.d2b-volume-acceptance.signingKey =
        volumeProviderArtifact.trustedPublisher.signingKey;
      d2b.zones.work.resources = {
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
        volume-operator = {
          type = "Role";
          spec.rules = [
            {
              resourceTypes = [
                "Endpoint"
                "Host"
                "Process"
                "Provider"
                "Volume"
                "VolumeBinding"
              ];
              verbs = [ "get" "list" ];
              subresources = [ ];
              resourceNames = [ ];
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
        volume-operator-binding = {
          type = "RoleBinding";
          spec = {
            roleRef = "Role/volume-operator";
            subjects = [ "User/alice" ];
            externalPrincipalSelector = null;
            scopeNarrowing = null;
          };
        };
        # The attachment execution target. The Nix bundle validation requires
        # attachment refs to resolve to a same-Zone Host or Guest; the
        # virtiofs serving path is host-side (the binding mints its socket
        # under /run/d2b/vms/<guest>/), so this fixture asserts the Volume
        # chain only - the Guest row itself stays declared input (KTD1).
        acceptance-guest = {
          type = "Guest";
          spec = {
            defaultDomain = "system";
            providerRef = "Provider/volume-virtiofs";
            budget = { };
            networkAttachments = [ ];
            deviceAttachments = [ ];
            volumeAttachmentDefaults = [ ];
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
        # The fixed daemon-owned Volume owner: volume-local is the only
        # Provider a Volume may select (U7 driver contract).
        volume-local = {
          type = "Provider";
          spec = {
            artifactId = "volume-acceptance-provider";
            config = {
              controllerExecutionRef = "Host/host-system";
              sourcePolicies = [
                {
                  id = "daemon-state";
                  class = "local-path";
                  volumeKinds = [ "durable" "state" "cache" ];
                }
              ];
            };
          };
        };
        # The serving Provider the derived VolumeBinding rows select and
        # whose signed virtiofsd-worker template the worker launch resolves.
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
              # Daemon-owned so the unprivileged daemon can provision the
              # local-path layout inline.
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
            # KTD1: the attachment stays declared input only. The Volume side
            # mints the durable VolumeBinding at reconcile; the deterministic
            # binding identity is derived from (volume, execution target,
            # view, mount path) - the fixture asserts that exact identity.
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
      };
      environment.systemPackages = with pkgs; [
        jq
        procps
      ];
    };
  };

  testScript = ''
    import hashlib
    import json
    import time

    # Deterministic identities the frozen v1 derivation pins (the fixture
    # asserts them, it does not invent them):
    # - the binding name derives from the attachment tuple;
    # - the private socket path derives from (zone, volume, guest) exactly as
    #   the daemon's serving effects resolve it.
    binding_name = "vol-binding-" + hashlib.sha256(
        b"d2b/volume-local/binding/v1\x00Volume/state"
        b"\x00Guest/acceptance-guest\x00controller\x00/state"
    ).hexdigest()[:24]
    socket_tag = hashlib.sha256(b"work\x00state\x00acceptance-guest").hexdigest()[:8]
    socket_path = f"/run/d2b/vms/acceptance-guest/vol-{socket_tag}.vfd.sock"

    # The API serves real row identities. A uid is not just non-null: the
    # daemon's `ResourceUid` Display is a redaction placeholder, and a
    # placeholder served through the read path would satisfy a null check
    # while breaking every consumer that parses the field (the operator
    # delete resolves its exact precondition uid from it).
    uuid_v4 = r"^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$"

    def d2b(command, out):
        return (
            "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock "
            f"d2b --zone work --json {command} >{out}"
        )

    start_all()
    machine.wait_for_unit("nftables.service", timeout=180)
    machine.wait_for_unit("d2b-broker.socket", timeout=30)
    machine.wait_for_unit("d2bd.service", timeout=180)
    machine.wait_for_file("/run/d2b/public.sock", timeout=30)

    # 1. Volume realize: the Nix-ingested Volume is Ready with a manager
    #    identity and an observed generation (U10 ingestion -> U7 driver).
    machine.wait_until_succeeds(
        f"{d2b('list Volume', '/run/d2b-volume-realized.json')} && "
        "jq -e '"
        "([.resources[] | select(.type == \"Volume\" and "
        ".metadata.name == \"state\")] | length) == 1 and "
        "(.resources[] | select(.type == \"Volume\" and "
        ".metadata.name == \"state\") | "
        f"((.metadata.uid | test(\"{uuid_v4}\")) and "
        ".metadata.generation > 0 and "
        ".metadata.ownerRef == null and "
        ".status.phase == \"Ready\" and "
        ".status.observedGeneration == .metadata.generation))' "
        "/run/d2b-volume-realized.json",
        timeout=180,
    )

    def dump_rows(tag):
        machine.succeed(
            d2b("list Volume", "/run/d2b-diag-volume.json") + "; "
            + d2b("list VolumeBinding", "/run/d2b-diag-binding.json") + "; "
            + d2b("list Process", "/run/d2b-diag-process.json") + "; "
            + d2b("list Endpoint", "/run/d2b-diag-endpoint.json") + "; echo OK"
        )
        print("=== volume-chain rows: " + tag)
        for kind in ["volume", "binding", "process", "endpoint"]:
            print(machine.succeed(
                "jq -c '[.resources[] | {type: .type, name: .metadata.name, "
                "owner: .metadata.ownerRef, phase: .status.phase, "
                "gen: .metadata.generation, obs: .status.observedGeneration, "
                "template: .spec.template, purpose: .spec.purpose, "
                "producer: .spec.producerRef}]' "
                "/run/d2b-diag-" + kind + ".json"
            ))
        print("=== socket dir:")
        print(machine.succeed(
            "ls -la /run/d2b/vms/acceptance-guest/ 2>&1 || echo 'no vms dir'"
        ))

    # 2. The Volume side minted exactly one deterministic VolumeBinding
    #    child through the manager, and that child converged.
    machine.wait_until_succeeds(
        f"{d2b('list VolumeBinding', '/run/d2b-binding-realized.json')} && "
        "jq -e '"
        f"([.resources[] | select(.type == \"VolumeBinding\" and "
        f".metadata.name == \"{binding_name}\" and "
        ".metadata.ownerRef == \"Volume/state\")] | length) == 1 and "
        f"(.resources[] | select(.metadata.name == \"{binding_name}\") | "
        f"((.metadata.uid | test(\"{uuid_v4}\")) and "
        ".metadata.generation > 0 and "
        ".status.phase == \"Ready\" and "
        ".status.observedGeneration == .metadata.generation and "
        ".spec.volumeRef == \"Volume/state\" and "
        ".spec.executionRef == \"Guest/acceptance-guest\" and "
        ".spec.view == \"controller\" and "
        ".spec.access == \"read-only\" and "
        ".spec.mountPath == \"/state\"))' "
        "/run/d2b-binding-realized.json",
        timeout=120,
    )

    # 3. The binding owns the virtiofsd worker Process and its private
    #    Endpoint as managed children; the worker is realized (a live
    #    virtiofsd process) and the serving socket is bound.
    machine.wait_until_succeeds(
        f"{d2b('list Process', '/run/d2b-worker-realized.json')} && "
        "jq -e '"
        f"([.resources[] | select(.type == \"Process\" and "
        f".metadata.ownerRef == \"VolumeBinding/{binding_name}\")] | length) "
        "== 1 and "
        f"([.resources[] | select(.type == \"Process\" and "
        f".metadata.ownerRef == \"VolumeBinding/{binding_name}\") | "
        "(.spec.providerRef == \"Provider/system-minijail\" and "
        ".spec.executionRef == \"Host/host-system\" and "
        ".spec.processClass == \"worker\" and "
        ".spec.template == \"virtiofsd-worker\" and "
        ".status.phase == \"Ready\" and "
        ".status.observedGeneration == .metadata.generation)] | length) == 1' "
        "/run/d2b-worker-realized.json",
        timeout=120,
    )
    try:
        machine.wait_until_succeeds(
            f"{d2b('list Endpoint', '/run/d2b-endpoint-realized.json')} && "
            f"{d2b('list Process', '/run/d2b-worker-producer.json')} && "
            "jq -e --slurpfile proc /run/d2b-worker-producer.json '"
            f"([.resources[] | select(.type == \"Endpoint\" and "
            f".metadata.ownerRef == \"VolumeBinding/{binding_name}\")] | length) "
            "== 1 and "
            f"([$proc[0].resources[] | select(.type == \"Process\" and "
            f".metadata.ownerRef == \"VolumeBinding/{binding_name}\" and "
            ".status.phase == \"Ready\" and "
            ".status.observedGeneration == .metadata.generation)] | length) "
            "== 1 and "
            f"(.resources[] | select(.type == \"Endpoint\" and "
            f".metadata.ownerRef == \"VolumeBinding/{binding_name}\") | "
            "(.status.phase == \"Ready\" and "
            ".status.observedGeneration == .metadata.generation and "
            ".spec.transport == \"unix\" and "
            "(.spec.purpose == \"virtiofsd\" and "
            "(.spec.producerRef as $producer | "
            "any($proc[0].resources[]; "
            ".type == \"Process\" and "
            "\"\\(.type)/\\(.metadata.name)\" == $producer)))))' "
            "/run/d2b-endpoint-realized.json",
            timeout=120,
        )
    except Exception:
        dump_rows("endpoint wait failed")
        raise
    machine.wait_until_succeeds(
        f"test -S {socket_path}",
        timeout=60,
    )
    machine.wait_until_succeeds(
        "test \"$(ps -eo args= | awk '/virtiofsd/ && !/awk/ {c++} END {print c+0}')\" "
        "-ge 1",
        timeout=60,
    )

    # 4. Deleting the owning Volume drives the whole chain through the
    #    preserved endpoint-first teardown: the binding is marked deleting
    #    first, its private Endpoint and socket are removed before the
    #    worker Process row, and nothing owned survives.
    machine.succeed(f"{d2b('list Volume', '/run/d2b-volume-pre-delete.json')}")
    volume_revision = machine.succeed(
        "jq -er '.resources[] | select(.type == \"Volume\" and "
        ".metadata.name == \"state\") | .metadata.revision' "
        "/run/d2b-volume-pre-delete.json"
    ).strip()
    machine.succeed(
        d2b("delete Volume/state --revision " + volume_revision,
            "/run/d2b-volume-delete.json")
    )

    try:
        machine.wait_until_succeeds(
            f"{d2b('list VolumeBinding', '/run/d2b-binding-deleting.json')} && "
            "jq -e '"
            f"any(.resources[]; .type == \"VolumeBinding\" and "
            f".metadata.name == \"{binding_name}\" and "
            ".metadata.deletionRequestedAt != null)' "
            "/run/d2b-binding-deleting.json",
            timeout=60,
        )
    except Exception:
        dump_rows("binding deleting wait failed")
        raise

    # Sample the teardown window: the Endpoint row and the worker Process
    # row must both disappear, the Endpoint never after the worker, and no
    # owned row may outlive its parent. The three lists are separate reads,
    # so they run parent -> worker -> endpoint: a parent observed gone ahead
    # of a child then really means the child was already gone when the
    # parent retired (the invariant under test), while the reverse order
    # could straddle the teardown and report a violation that never held.
    binding_owner = "VolumeBinding/" + binding_name
    sample_expr = (
        "socket=false; test -S " + socket_path + " && socket=true; "
        "jq -n --argjson socket \"$socket\" "
        "--slurpfile b /run/d2b-teardown-binding.json "
        "--slurpfile p /run/d2b-teardown-process.json "
        "--slurpfile e /run/d2b-teardown-endpoint.json "
        "'{"
        "endpoint: ([$e[0].resources[] | "
        "select(.metadata.ownerRef == \"" + binding_owner + "\")] | length), "
        "worker: ([$p[0].resources[] | select(.type == \"Process\" and "
        ".metadata.ownerRef == \"" + binding_owner + "\")] | length), "
        "binding: ([$b[0].resources[] | select(.type == \"VolumeBinding\" "
        "and .metadata.name == \"" + binding_name + "\")] | length), "
        "socket: $socket"
        "}'"
    )
    observed = []
    for _ in range(600):
        machine.succeed(
            d2b("list VolumeBinding", "/run/d2b-teardown-binding.json") + "; "
            + d2b("list Process", "/run/d2b-teardown-process.json") + "; "
            + d2b("list Endpoint", "/run/d2b-teardown-endpoint.json") + "; "
            + "echo OK"
        )
        observed.append(
            json.loads(machine.succeed(sample_expr))
        )
        last = observed[-1]
        if last["binding"] == 0 and last["endpoint"] == 0 and last["worker"] == 0:
            break
        time.sleep(0.2)
    else:
        dump_rows("teardown did not converge")
        raise AssertionError(
            "volume teardown did not converge within its budget: "
            + json.dumps(observed[-1])
        )

    for sample in observed:
        if sample["binding"] == 0:
            assert sample["endpoint"] == 0 and sample["worker"] == 0, (
                f"owned child outlived its parent binding: {sample}"
            )
        if sample["worker"] == 0:
            assert sample["endpoint"] == 0, (
                f"worker Process row disappeared before the binding-owned "
                f"Endpoint row (endpoint-first teardown violated): {sample}"
            )

    machine.succeed(
        f"{d2b('list Volume', '/run/d2b-volume-after-delete.json')} && "
        "jq -e 'all(.resources[]; "
        "(.type == \"Volume\" and .metadata.name == \"state\") | not)' "
        "/run/d2b-volume-after-delete.json"
    )
  '';
}
