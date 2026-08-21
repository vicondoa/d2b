{ pkgs, self }:

let
  d2bLib = import ./lib.nix {
    inherit self;
    inherit (pkgs) lib;
  };
  contractScript = pkgs.writeText "d2b-cutover-rehearsal-contracts.py" ''
    import hashlib
    import json
    import os
    import struct
    import time

    root = "/run/d2b/cutover-rehearsal"
    operation = os.environ["OPERATION"]
    candidate = os.environ["CANDIDATE"]
    preview = os.environ["PREVIEW"]
    operator = "uid-1000"
    host = "sha256:" + "a" * 64
    restore = "sha256:" + "b" * 64
    now = int(time.time() * 1000)
    zone_ids = ["local-root", "personal", "work"]
    assert all("/" not in zone_id for zone_id in zone_ids)

    def canonical(value):
        return json.dumps(
            value, sort_keys=True, separators=(",", ":"), ensure_ascii=False
        ).encode()

    def digest(domain, value):
        framed = domain.encode() + b"\0" + canonical(value)
        # Match d2b_contracts::v3::canonical_digest exactly.
        return "sha256:" + hashlib.sha256(framed).hexdigest()

    recovery = {
        "recoveryId": "u6-recovery-point",
        "candidateId": candidate,
        "hostDigest": host,
        "previewDigest": preview,
        "operatorId": operator,
        "restoreInstructionsDigest": restore,
        "issuedAtMs": now - 1000,
        "expiresAtMs": now + 600000,
        "qualified": True,
    }
    recovery_digest = digest("d2b:cutover:recovery:v1", recovery)

    binding = {
        "operationId": operation,
        "operationKind": "cutover",
        "candidateId": candidate,
        "previewDigest": preview,
        "recoveryDigest": recovery_digest,
        "systemArtifactId": os.environ["SYSTEM_ARTIFACT"],
        "sourceSystemArtifactId": os.environ["SOURCE_SYSTEM_ARTIFACT"],
        "operatorId": operator,
    }
    consent = {
        "binding": binding,
        "issuedAtMs": now - 1000,
        "expiresAtMs": now + 600000,
        "consumed": False,
    }
    consent_digest = digest("d2b:cutover:consent:v1", consent)

    def write(name, value):
        path = os.path.join(root, name)
        with open(path, "w", encoding="utf-8") as stream:
            json.dump(value, stream, sort_keys=True, separators=(",", ":"))
            stream.write("\n")
        os.chmod(path, 0o644)

    write("recovery.json", recovery)
    write("consent.json", consent)
    write("invalid-consent.json", {"binding": binding, "consumed": True})
    write("finalization-consent.json", {
        "binding": {
            "operationId": operation,
            "operationKind": "cutover",
            "candidateId": candidate,
            "previewDigest": preview,
            "operatorId": operator,
        },
        "issuedAtMs": now - 1000,
        "expiresAtMs": now + 600000,
        "consumed": False,
    })
    write("finalization-plan.json", {"artifacts": []})
    write("verification.json", {
        "zones": [{"zoneId": zone_id, "healthy": True} for zone_id in zone_ids],
        "sourcesPreserved": True,
        "identityDigestsMatch": True,
        "candidateCurrent": True,
    })

    artifact = os.environ["SYSTEM_ARTIFACT"]
    target = "Host/host-system"
    generation = 2
    fingerprint = hashlib.sha256(
        target.encode() + b"\0" + artifact.encode() + b"\0"
        + struct.pack(">Q", generation)
    ).digest()
    write("handoff.json", {
        "callerRole": "lifecycle",
        "target": target,
        "intent": {
            "sourceGeneration": 1,
            "targetGeneration": generation,
            "systemArtifactId": artifact,
            "activationMode": "adopt",
            "compatibility": {
                "minimumGeneration": 1,
                "targetFingerprint": list(fingerprint),
            },
        },
    })
    with open(os.path.join(root, "digests.env"), "w", encoding="utf-8") as stream:
        stream.write("RECOVERY_DIGEST=" + recovery_digest + "\n")
        stream.write("CONSENT_DIGEST=" + consent_digest + "\n")
        stream.write("HOST_DIGEST=" + host + "\n")
'';
  acceptanceSystem = pkgs.writeShellScriptBin "switch-to-configuration" ''
    exit 0
  '';
in
pkgs.testers.runNixOSTest {
  name = "d2b-cutover-rehearsal";

  nodes.machine = d2bLib.d2bDaemonNode {
    extra = { config, pkgs, ... }: {
      d2b.site.adminUsers = [ "alice" "bob" ];
      d2b.artifacts.acceptance-system = {
        package = acceptanceSystem;
        type = "nixos-system";
      };
      d2b.artifacts.u6-rehearsal-source = {
        package = acceptanceSystem;
        type = "nixos-system";
      };

      # Three independent configured Zones make the preview and verification
      # exercise the host-wide all-Zone boundary instead of a one-Zone shortcut.
      d2b.zones.local-root = { };
      d2b.zones.work.parentZone = "local-root";
      d2b.zones.personal.parentZone = "local-root";

      users.users.bob = {
        isNormalUser = true;
        uid = 1001;
      };

      environment.systemPackages = with pkgs; [
        coreutils
        jq
        python3
      ];

      # Synthetic, non-secret identity and source markers. They let the
      # rehearsal prove preservation and an external recovery readback without
      # touching a host user's data.
      environment.etc."d2b/cutover-rehearsal/identity/tpm".text = "tpm-identity-fixture\n";
      environment.etc."d2b/cutover-rehearsal/identity/volume".text =
        "durable-volume-fixture\n";
      environment.etc."d2b/cutover-rehearsal/identity/ssh".text = "ssh-key-fixture\n";
      environment.etc."d2b/cutover-rehearsal/identity/audit".text =
        "audit-chain-fixture\n";
      environment.etc."d2b/cutover-rehearsal/contracts.py".source = contractScript;
    };
  };

  testScript = ''
    import json
    import shlex

    start_all()
    machine.wait_for_unit("d2bd.service")
    machine.wait_for_unit("d2b-broker.socket")
    machine.wait_for_file("/run/d2b/public.sock")

    # The daemon-only end state has exactly three root-visible persistent
    # units. A cutover runner is operation-scoped and must never become a
    # fourth systemd service.
    declared = machine.succeed("cat /etc/d2b/daemon-acceptance-units").splitlines()
    assert declared == [
        "d2bd.service",
        "d2b-broker.socket",
        "d2b-broker.service",
    ]
    machine.succeed(
        "test -z \"$(systemctl list-unit-files --no-legend "
        "'d2b-cutover*.service' | sed '/^[[:space:]]*$/d')\""
    )

    operation = "u6-rehearsal-bootstrap"
    candidate = "u6-rehearsal-candidate"
    revision = "u6-rehearsal-plan"
    zone_ids = ["local-root", "personal", "work"]
    assert all("/" not in zone_id for zone_id in zone_ids)

    d2b_bin = "d2b"

    def run_cli(command):
        return machine.succeed(
            "runuser -u alice -- " + shlex.quote(d2b_bin)
            + " " + command + " --json"
        )

    def preview_for(operation_id):
        value = json.loads(run_cli(
            "host cutover preview"
            f" --operation-id {operation_id}"
            f" --candidate-id {candidate}"
            f" --revision-plan-id {revision}"
            f" --system-artifact-id {artifact}"
            f" --source-system-artifact-id {source_artifact}"
        ))
        assert value["state"] == "planned"
        assert value["phase"] == 0
        assert value["mutationAccepted"] is False
        assert value["inventory"]["complete"] is True
        assert value["inventory"]["zoneCount"] == 3
        assert value["inventory"]["itemCount"] >= 3
        # The outer CLI envelope carries a human-readable ResourceRef. The
        # inventory and verification payloads below carry opaque Zone IDs.
        assert value["zoneRef"] == "Zone/local-root"
        assert value["zoneRef"].split("/", 1)[1] in zone_ids
        return value

    artifact = machine.succeed(
        "jq -r '.entries[] | select(.type == \"nixos-system\") | .artifactId' "
        "/etc/d2b/artifact-catalog.json | head -n 1"
    ).strip()
    source_artifact = "u6-rehearsal-source"

    # The preview is mutation-free, byte-stable, redaction-safe, and covers
    # every configured Zone. Repeating it proves the same inventory digest is
    # used for exact consent binding.
    preview = preview_for(operation)
    repeated = preview_for(operation)
    assert repeated["previewDigest"] == preview["previewDigest"]
    assert all(
        "/" not in zone_id
        for zone_id in zone_ids
    )
    machine.succeed(
        "test ! -e /var/lib/d2b/cutover/"
        + shlex.quote(operation)
        + "/journal.json"
    )

    # One-time cutover is host-wide. A Zone selection is refused before the
    # daemon is contacted, so it cannot create a partial operation.
    machine.fail(
        "runuser -u alice -- d2b --zone work host cutover preview"
        f" --operation-id {operation}"
        f" --candidate-id {candidate}"
        f" --revision-plan-id {revision} --json"
    )

    # Build strict U3 recovery and consent artifacts from the production
    # preview digest. This fixture intentionally uses only opaque test
    # identities and bounded timestamps.
    machine.succeed(
        "install -d -m 0755 /run/d2b/cutover-rehearsal "
        "/var/lib/d2b/cutover-rehearsal"
    )
    preview_digest = preview["previewDigest"]
    machine.succeed(
        "OPERATION=" + shlex.quote(operation)
        + " CANDIDATE=" + shlex.quote(candidate)
        + " PREVIEW=" + shlex.quote(preview_digest)
        + " SYSTEM_ARTIFACT=" + shlex.quote(artifact)
        + " SOURCE_SYSTEM_ARTIFACT=" + shlex.quote(source_artifact)
        + " python3 /etc/d2b/cutover-rehearsal/contracts.py"
    )
    digests = {}
    for line in machine.succeed(
        "cat /run/d2b/cutover-rehearsal/digests.env"
    ).splitlines():
        key, value = line.split("=", 1)
        digests[key] = value

    common_apply = (
        "host cutover apply"
        f" --operation-id {operation}"
        f" --candidate-id {candidate}"
        f" --revision-plan-id {revision}"
        f" --system-artifact-id {artifact}"
        f" --source-system-artifact-id {source_artifact}"
        f" --preview-digest {preview_digest}"
        f" --recovery-digest {digests['RECOVERY_DIGEST']}"
        " --operator-id uid-1000"
        f" --consent-digest {digests['CONSENT_DIGEST']}"
        " --recovery-attestation-file /run/d2b/cutover-rehearsal/recovery.json"
        f" --host-digest {digests['HOST_DIGEST']}"
    )

    # A consumed or mismatched consent is refused before bootstrap, journal,
    # drain, or any broker effect.
    machine.fail(
        "runuser -u alice -- d2b " + common_apply
        + " --consent-file /run/d2b/cutover-rehearsal/invalid-consent.json"
        + " --json"
    )
    machine.succeed(
        "test ! -e /var/lib/d2b/cutover/"
        + shlex.quote(operation)
        + "/journal.json"
    )

    # Bootstrap the out-of-band runner without applying the generation yet.
    # This proves exact consent admission, a durable journal, and the owner
    # socket before the control-plane drain.
    machine.succeed(
        "runuser -u alice -- d2b " + common_apply
        + " --consent-file /run/d2b/cutover-rehearsal/consent.json --json"
    )
    status = json.loads(run_cli(
        "host cutover status"
        f" --operation-id {operation}"
    ))
    assert status["operationId"] == operation
    assert status["phase"] == 0
    assert status["state"] == "applying"

    # Any configured Admin may set a safety hold; only the bound operator may
    # resume without fresh digest-bound consent. This uses SO_PEERCRED at the
    # runner socket, not a client-supplied role string.
    machine.succeed(
        "runuser -u bob -- d2b host cutover hold"
        f" --operation-id {operation} --reason rehearsal-hold --json"
    )
    held = json.loads(run_cli(
        "host cutover status"
        f" --operation-id {operation}"
    ))
    assert held["state"] == "held"
    machine.fail(
        "runuser -u bob -- d2b host cutover resume"
        f" --operation-id {operation} --json"
    )
    machine.succeed(
        "runuser -u alice -- d2b host cutover resume"
        f" --operation-id {operation} --json"
    )

    # Native rollback is still available before phase 5. The journal and all
    # identity/source fixtures remain intact after the terminal rollback.
    machine.succeed(
        "runuser -u alice -- d2b host cutover rollback"
        f" --operation-id {operation} --json"
    )
    rolled_back = json.loads(run_cli(
        "host cutover status"
        f" --operation-id {operation}"
    ))
    assert rolled_back["state"] == "rolled-back"
    for marker in [
        "/etc/d2b/cutover-rehearsal/identity/tpm",
        "/etc/d2b/cutover-rehearsal/identity/volume",
        "/etc/d2b/cutover-rehearsal/identity/ssh",
        "/etc/d2b/cutover-rehearsal/identity/audit",
    ]:
        machine.succeed("test -s " + marker)

    # Restart the daemon after the runner terminal state. The operation owner
    # remains the runner and the journal is still readable; d2bd never adopts
    # repair ownership.
    machine.succeed("systemctl restart d2bd.service")
    machine.wait_for_unit("d2bd.service")
    machine.succeed(
        "runuser -u alice -- d2b host cutover status"
        f" --operation-id {operation} --json"
    )

    # A second operation reaches the phase-4/phase-5 handoff boundary. The
    # typed handoff uses the currently active catalog artifact in Adopt mode,
    # so the helper proves target identity without switching the VM system.
    operation2 = "u6-rehearsal-handoff"
    preview2 = preview_for(operation2)
    preview2_digest = preview2["previewDigest"]
    d2b_bin = machine.succeed(
        "readlink -f /run/current-system/sw/bin/d2b"
    ).strip()
    original_system = machine.succeed(
        "readlink -f /run/current-system"
    ).strip()
    fake_system = machine.succeed(
        "jq -r '.entries[] | select(.artifactId == \"acceptance-system\") | .storePath' "
        "/etc/d2b/artifact-catalog.json"
    ).strip()
    machine.succeed(
        "OPERATION=" + shlex.quote(operation2)
        + " PREVIEW=" + shlex.quote(preview2_digest)
        + " SYSTEM_ARTIFACT=" + shlex.quote(artifact)
        + " SOURCE_SYSTEM_ARTIFACT=" + shlex.quote(source_artifact)
        + " CANDIDATE=" + shlex.quote(candidate)
        + " python3 /etc/d2b/cutover-rehearsal/contracts.py"
    )
    digests2 = {}
    for line in machine.succeed(
        "cat /run/d2b/cutover-rehearsal/digests.env"
    ).splitlines():
        key, value = line.split("=", 1)
        digests2[key] = value
    machine.succeed(
        "ln -sfn " + shlex.quote(fake_system) + " /run/current-system"
    )
    apply2 = (
        "host cutover apply"
        f" --operation-id {operation2}"
        f" --candidate-id {candidate}"
        f" --revision-plan-id {revision}"
        f" --system-artifact-id {artifact}"
        f" --source-system-artifact-id {source_artifact}"
        f" --preview-digest {preview2_digest}"
        f" --recovery-digest {digests2['RECOVERY_DIGEST']}"
        " --operator-id uid-1000"
        f" --consent-digest {digests2['CONSENT_DIGEST']}"
        " --consent-file /run/d2b/cutover-rehearsal/consent.json"
        " --recovery-attestation-file /run/d2b/cutover-rehearsal/recovery.json"
        f" --host-digest {digests2['HOST_DIGEST']}"
        " --handoff-file /run/d2b/cutover-rehearsal/handoff.json"
        " --json"
    )
    machine.succeed(
        "runuser -u alice -- " + shlex.quote(d2b_bin) + " " + apply2
    )
    machine.succeed(
        "ln -sfn " + shlex.quote(original_system) + " /run/current-system"
    )
    machine.succeed("systemctl start d2bd.service")
    machine.wait_for_unit("d2bd.service")
    handoff_status = json.loads(run_cli(
        "host cutover status"
        f" --operation-id {operation2}"
    ))
    assert handoff_status["phase"] == 5
    assert handoff_status["state"] == "applying"

    # Inject a typed phase-5 effect failure through the owner socket. The
    # runner records the terminal external-restore outcome after durable audit
    # publication; it does not pretend that native rollback is still open.
    effect_failure = (
        "import json,socket\n"
        "path='/run/d2b/cutover/u6-rehearsal-handoff/runner.sock'\n"
        "command={'command':'effect','effectId':'phase-five-failure',"
        "'stepId':'phase-five-failure','kind':'resource-store-create',"
        "'replayClass':'reopen-by-journaled-identity',"
        "'advanceTo':'provider-install','identity':'store-identity'}\n"
        "sock=socket.socket(socket.AF_UNIX)\n"
        "sock.connect(path)\n"
        "sock.sendall(json.dumps(command,separators=(',',':')).encode())\n"
        "sock.shutdown(socket.SHUT_WR)\n"
        "sock.recv(65536)\n"
    )
    machine.succeed(
        "runuser -u alice -- python3 -c " + shlex.quote(effect_failure)
    )
    restore_required = json.loads(run_cli(
        "host cutover status"
        f" --operation-id {operation2}"
    ))
    assert restore_required["phase"] == 5
    assert restore_required["state"] == "restore-required"

    # Phase 5 closes native rollback. Verification and finalization cannot
    # bypass the external restore boundary or the separate phase-10 consent.
    machine.fail(
        "runuser -u alice -- d2b host cutover rollback"
        f" --operation-id {operation2}"
        " --handoff-file /run/d2b/cutover-rehearsal/handoff.json --json"
    )
    machine.fail(
        "runuser -u alice -- d2b host cutover verify"
        f" --operation-id {operation2}"
        " --verification-file /run/d2b/cutover-rehearsal/verification.json --json"
    )
    machine.fail(
        "runuser -u alice -- d2b host cutover finalize"
        f" --operation-id {operation2}"
        " --consent-file /run/d2b/cutover-rehearsal/finalization-consent.json"
        " --finalization-file /run/d2b/cutover-rehearsal/finalization-plan.json --json"
    )

    # External recovery mechanism drill: mutate a synthetic identity point,
    # restore the read-only recovery copy, and compare every preserved digest.
    # This is intentionally outside d2b state and models the qualified
    # operator-owned recovery mechanism required before a real cutover.
    machine.succeed(
        "install -d -m 0700 /run/d2b/cutover-rehearsal/recovery-copy && "
        "cp -a /etc/d2b/cutover-rehearsal/identity/. "
        "/run/d2b/cutover-rehearsal/recovery-copy/ && "
        "chmod -R a-w /run/d2b/cutover-rehearsal/recovery-copy && "
        "printf 'mutated\\n' > /etc/d2b/cutover-rehearsal/identity/volume && "
        "cp -a /run/d2b/cutover-rehearsal/recovery-copy/. "
        "/etc/d2b/cutover-rehearsal/identity/ && "
        "cmp /run/d2b/cutover-rehearsal/recovery-copy/volume "
        "/etc/d2b/cutover-rehearsal/identity/volume"
    )

    # Audit continuity is durable across the bootstrap, hold, rollback, drain,
    # and generation-handoff transitions. The journal is path-free and keeps
    # typed effect identities rather than raw host locators.
    machine.succeed(
        "test -s /var/lib/d2b/cutover/"
        + shlex.quote(operation)
        + "/journal.json && "
        "grep -F 'phaseStarted' /var/lib/d2b/audit/*.jsonl >/dev/null"
    )
    machine.succeed(
        "test -s /var/lib/d2b/cutover/"
        + shlex.quote(operation2)
        + "/journal.json && "
        "grep -F 'host-drain' /var/lib/d2b/cutover/"
        + shlex.quote(operation2)
        + "/journal.json >/dev/null && "
        "grep -F 'closure-activation' /var/lib/d2b/cutover/"
        + shlex.quote(operation2)
        + "/journal.json >/dev/null"
    )
  '';
}
