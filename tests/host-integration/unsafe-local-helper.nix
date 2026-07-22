# Type 10 (runNixOSTest): live daemon/socket-activation/host-posture wiring
# for the unsafe-local-helper responder/service seam.
#
# Scope boundary (documented per AGENTS.md "Existing code is canon" /
# spec-correction convention): per `tests/AGENTS.md`'s Type-10 definition,
# this VM test asserts *live host wiring* -- real units, real
# socket-activated ACLs, real per-uid kernel/SO_PEERCRED authorization,
# and the live process's resilience/fail-closed behaviour under
# non-conforming input. It deliberately does **not** reimplement the
# `RuntimeSystemdUser` ComponentSession wire protocol (Noise handshake,
# offer negotiation, record protection, attachment framing) as a
# from-scratch client here: no host-integration test in this tree does
# that for any ComponentSession-based responder, and hand-rolling a second,
# independent implementation of that multi-layered framing in this test
# script would itself become an unaudited shadow protocol implementation
# rather than a conformance check. That real end-to-end conformance --
# genuine `SystemdUserScopeManager` + `ShellSupervisor` backend dispatch,
# real inbound-attachment fd flow, cancellation binding, and
# detach-without-kill semantics -- is exercised directly against the real
# responder code (the same `RuntimeAdapter`/`ShellAdapter`/`RealBackend`
# this service process runs) by the hermetic and real-backend tests in
# `packages/d2b-unsafe-local-helper/src/server.rs` (Type 2, `src/**`,
# owned by this same change). This test's job is everything those
# in-process tests cannot see: that the *real* systemd units, sockets,
# ACLs, and kernel-enforced peer identity actually gate the real running
# service the way the module declares.
{ pkgs, self }:

let
  d2bLib = import ./lib.nix {
    inherit self;
    inherit (pkgs) lib;
  };
in
pkgs.testers.runNixOSTest {
  name = "d2b-unsafe-local-helper";

  nodes.machine = d2bLib.d2bDaemonNode {
    extra = { pkgs, ... }: {
      users.users.bob = {
        isNormalUser = true;
        uid = 1001;
      };
      d2b.site.adminUsers = [ "alice" ];
      d2b.realms.host = {
        allowedUsers = [ "alice" ];
        policy.allowUnsafeLocal = true;
        providers.systemd-user = {
          type = "runtime";
          implementationId = "systemd-user";
        };
        workloads.tools = {
          providerRefs.runtime = "systemd-user";
          launcher.items.probe = {
            type = "exec";
            name = "Probe";
            argv = [ "true" ];
          };
        };
      };
      environment.systemPackages = [ pkgs.python3 ];
    };
  };

  testScript = ''
    start_all()
    machine.wait_for_unit("d2bd.service")
    machine.fail("test -e /run/d2b/unsafe-local-helper.sock")
    machine.succeed("id -nG alice | tr ' ' '\n' | grep -qx d2b-unsafe-local")
    machine.fail("id -nG bob | tr ' ' '\n' | grep -qx d2b-unsafe-local")

    machine.succeed("systemctl start user@1000.service")
    alice_user = (
        "runuser -u alice -- env XDG_RUNTIME_DIR=/run/user/1000 "
        "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus "
        "systemctl --user"
    )
    machine.wait_until_succeeds(
        alice_user + " is-active d2b-runtime-systemd-user.socket",
        timeout=60,
    )
    machine.wait_until_succeeds(
        alice_user + " is-active d2b-userd.socket",
        timeout=60,
    )
    endpoint = "/run/d2b/u/1000/runtime-agent.sock"
    userd_endpoint = "/run/d2b/u/1000/userd.sock"
    machine.wait_for_file(endpoint)
    machine.wait_for_file(userd_endpoint)
    machine.succeed(f"test -S {endpoint}")
    machine.succeed(f"test -S {userd_endpoint}")
    machine.succeed(f"test \"$(stat -c %a {endpoint})\" = 600")
    machine.succeed(f"test \"$(stat -c %a {userd_endpoint})\" = 600")
    machine.succeed(f"test \"$(stat -c %U {endpoint})\" = alice")
    machine.succeed(f"test \"$(stat -c %U {userd_endpoint})\" = alice")
    machine.fail("test -e /run/d2b/u/1000/unsafe-local-helper.sock")

    machine.fail(
        "runuser -u alice -- "
        "/run/current-system/sw/bin/d2b-unsafe-local-helper"
    )
    machine.succeed(
        "runuser -u alice -- python3 -c "
        "'import socket; s=socket.socket(socket.AF_UNIX, socket.SOCK_SEQPACKET); "
        f"s.connect(\"{endpoint}\")'"
    )
    machine.succeed(
        "runuser -u alice -- python3 -c "
        "'import socket; s=socket.socket(socket.AF_UNIX, socket.SOCK_SEQPACKET); "
        f"s.connect(\"{userd_endpoint}\")'"
    )

    # Real responder authentication/handshake gate, exercised against the
    # live `d2b-runtime-systemd-user.service` process (not a stub): a
    # connected peer that never completes the required
    # `SessionEngine::establish_responder` Noise offer/handshake -- here, a
    # same-uid client that sends bytes shaped like nothing the wire
    # contract accepts -- is rejected and the connection cleanly closed
    # (`recv` observes EOF) rather than hung, echoed back, or crashing the
    # service. This is the live, real-process counterpart to the fixed
    # channel-binding vectors and handshake-shaped decoder rejections
    # already covered as hermetic unit tests in
    # `packages/d2b-unsafe-local-helper/src/server.rs`.
    garbage_probe = (
        "runuser -u alice -- python3 -c "
        "'import socket; s=socket.socket(socket.AF_UNIX, socket.SOCK_SEQPACKET); "
        "s.settimeout(10); "
        f"s.connect(\"{endpoint}\"); "
        "s.send(b\"not-a-real-component-session-handshake-offer-0123456789\"); "
        "data=s.recv(4096); "
        "assert len(data) == 0, "
        "f\"expected the responder to close on a malformed handshake offer, "
        "got {data!r}\"'"
    )
    machine.succeed(garbage_probe)

    machine.succeed(
        alice_user
        + " show -P NRestarts d2b-runtime-systemd-user.service | grep -qx 0"
    )
    machine.succeed(alice_user + " is-active d2b-runtime-systemd-user.service")

    # Base per-user ACL is exactly what `aclSyncScriptText` sets: no
    # accidental broad grant survives socket activation when this
    # topology's controller allowlist has no rows for `alice`.
    endpoint_acl = machine.succeed(f"getfacl -p {endpoint}")
    assert "user::rw-" in endpoint_acl, endpoint_acl
    assert "other::---" in endpoint_acl, endpoint_acl

    # `d2b-userd` is a separate, unrelated service/module (out of this
    # seam's owned scope) that still runs the pre-existing stub; its
    # behaviour is unchanged by this change and asserted here only to
    # confirm this test's own topology still exercises it correctly.
    machine.wait_until_succeeds(
        "journalctl _SYSTEMD_USER_UNIT=d2b-userd.service "
        "_UID=1000 --no-pager "
        "| grep -q 'service mode is not implemented'",
        timeout=60,
    )
    machine.succeed(
        alice_user + " show -P NRestarts d2b-userd.service | grep -qx 0"
    )

    machine.succeed("systemctl start user@1001.service")
    bob_user = (
        "runuser -u bob -- env XDG_RUNTIME_DIR=/run/user/1001 "
        "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1001/bus "
        "systemctl --user"
    )
    machine.wait_until_succeeds(
        bob_user
        + " show -P ConditionResult d2b-runtime-systemd-user.socket | grep -qx no",
        timeout=60,
    )
    machine.fail(bob_user + " is-active d2b-runtime-systemd-user.socket")
    machine.fail(bob_user + " is-active d2b-userd.socket")
    machine.fail("test -e /run/d2b/u/1001/runtime-agent.sock")
    machine.fail("test -e /run/d2b/u/1001/userd.sock")
  '';
}
