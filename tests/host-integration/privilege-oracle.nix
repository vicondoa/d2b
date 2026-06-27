# Type-G runNixOSTest: live broker privilege posture oracle.
#
# Hermetic successor to the retired self-hosted L1c shell oracle. It boots a
# d2b daemon host, starts the socket-activated privileged broker, derives the
# expected posture from the rendered systemd unit, and checks the live
# /proc/<pid> state for the hardening invariants that matter at runtime.
{ pkgs, self }:

let
  d2bLib = import ./lib.nix {
    inherit self;
    inherit (pkgs) lib;
  };
in
pkgs.testers.runNixOSTest {
  name = "d2b-privilege-oracle";

  nodes.machine = d2bLib.d2bDaemonNode { };

  testScript = ''
    import shlex

    start_all()

    machine.wait_for_unit("d2b-priv-broker.socket")
    machine.wait_for_unit("d2bd.service")

    # The broker is socket-activated, but starting the service directly keeps a
    # live Type=notify process long enough to read its /proc posture.
    machine.succeed("systemctl start d2b-priv-broker.service")
    broker_pid = machine.succeed(
        "for i in $(seq 1 100); do "
        "pid=$(systemctl show -p MainPID --value d2b-priv-broker.service); "
        "if [ -n \"$pid\" ] && [ \"$pid\" != 0 ] && [ -r \"/proc/$pid/status\" ]; then "
        "echo \"$pid\"; exit 0; fi; "
        "sleep 0.2; "
        "done; "
        "systemctl status --no-pager d2b-priv-broker.service >&2; "
        "exit 1"
    ).strip()
    print(f"live d2b-priv-broker PID: {broker_pid}")

    unit_raw = machine.succeed(
        "systemctl show d2b-priv-broker.service "
        "-p CapabilityBoundingSet "
        "-p AmbientCapabilities "
        "-p NoNewPrivileges "
        "-p User "
        "-p Group "
        "-p Slice "
        "-p SystemCallFilter"
    )
    print("rendered d2b-priv-broker.service posture:\n" + unit_raw)
    unit = dict(line.split("=", 1) for line in unit_raw.strip().splitlines() if "=" in line)

    status_raw = machine.succeed(f"cat /proc/{broker_pid}/status")
    cgroup_raw = machine.succeed(f"cat /proc/{broker_pid}/cgroup")
    ns_raw = machine.succeed(
        f"for ns in cgroup ipc mnt net pid time time_for_children user uts; do "
        f"[ -e /proc/{broker_pid}/ns/$ns ] && printf '%s=%s\\n' \"$ns\" \"$(readlink /proc/{broker_pid}/ns/$ns)\"; "
        "done"
    )
    print("live /proc status subset:\n" + "\n".join(
        line for line in status_raw.splitlines()
        if line.startswith(("Uid:", "Gid:", "Groups:", "CapEff:", "CapBnd:", "CapAmb:", "NoNewPrivs:", "Seccomp:"))
    ))
    print("live cgroup:\n" + cgroup_raw)
    print("live namespaces:\n" + ns_raw)

    status = {}
    for line in status_raw.splitlines():
        if ":" in line:
            key, value = line.split(":", 1)
            status[key] = value.strip()

    cap_names = [
        "CHOWN",
        "DAC_OVERRIDE",
        "DAC_READ_SEARCH",
        "FOWNER",
        "FSETID",
        "KILL",
        "SETGID",
        "SETUID",
        "SETPCAP",
        "LINUX_IMMUTABLE",
        "NET_BIND_SERVICE",
        "NET_BROADCAST",
        "NET_ADMIN",
        "NET_RAW",
        "IPC_LOCK",
        "IPC_OWNER",
        "SYS_MODULE",
        "SYS_RAWIO",
        "SYS_CHROOT",
        "SYS_PTRACE",
        "SYS_PACCT",
        "SYS_ADMIN",
        "SYS_BOOT",
        "SYS_NICE",
        "SYS_RESOURCE",
        "SYS_TIME",
        "SYS_TTY_CONFIG",
        "MKNOD",
        "LEASE",
        "AUDIT_WRITE",
        "AUDIT_CONTROL",
        "SETFCAP",
        "MAC_OVERRIDE",
        "MAC_ADMIN",
        "SYSLOG",
        "WAKE_ALARM",
        "BLOCK_SUSPEND",
        "AUDIT_READ",
        "PERFMON",
        "BPF",
        "CHECKPOINT_RESTORE",
    ]
    cap_numbers = {name: index for index, name in enumerate(cap_names)}
    cap_last_cap = int(machine.succeed("cat /proc/sys/kernel/cap_last_cap").strip())
    full_cap_mask = (1 << (cap_last_cap + 1)) - 1

    def parse_cap_set(value, *, empty_is_full):
        value = value.strip()
        if value == "":
            return full_cap_mask if empty_is_full else 0
        if value.lower().startswith("0x"):
            return int(value, 16)

        mask = 0
        for token in value.split():
            token = token.strip()
            if not token:
                continue
            norm = token.upper().replace("-", "_")
            if norm.startswith("CAP_"):
                norm = norm[4:]
            assert norm in cap_numbers, f"unknown capability from systemd unit: {token}"
            bit = cap_numbers[norm]
            assert bit <= cap_last_cap, (
                f"systemd unit declares capability {token} above kernel cap_last_cap={cap_last_cap}"
            )
            mask |= 1 << bit
        return mask

    def parse_unit_bool(value):
        norm = value.strip().lower()
        if norm in ("yes", "true", "1"):
            return 1
        if norm in ("no", "false", "0", ""):
            return 0
        raise AssertionError(f"unknown systemd boolean value: {value!r}")

    expected_uid = int(machine.succeed(f"id -u {shlex.quote(unit['User'])}").strip())
    expected_gid = int(
        machine.succeed(f"getent group {shlex.quote(unit['Group'])} | cut -d: -f3").strip()
    )
    expected_cap_bnd = parse_cap_set(unit["CapabilityBoundingSet"], empty_is_full=True)
    expected_cap_amb = parse_cap_set(unit["AmbientCapabilities"], empty_is_full=False)
    expected_nonewprivs = parse_unit_bool(unit["NoNewPrivileges"])
    expected_slice = unit["Slice"].strip()

    actual_uids = [int(part) for part in status["Uid"].split()]
    actual_gids = [int(part) for part in status["Gid"].split()]
    actual_cap_eff = int(status["CapEff"], 16)
    actual_cap_bnd = int(status["CapBnd"], 16)
    actual_cap_amb = int(status["CapAmb"], 16)
    actual_nonewprivs = int(status["NoNewPrivs"])
    actual_seccomp = int(status["Seccomp"])
    cgroup_paths = [line.split(":", 2)[2] for line in cgroup_raw.splitlines() if ":" in line]

    assert all(uid == expected_uid for uid in actual_uids), (
        f"broker Uid must match rendered User={unit['User']} ({expected_uid}), got {actual_uids}"
    )
    assert expected_uid == 0, f"broker must run as root uid 0, rendered User={unit['User']}"
    assert all(gid == expected_gid for gid in actual_gids), (
        f"broker Gid must match rendered Group={unit['Group']} ({expected_gid}), got {actual_gids}"
    )

    assert actual_cap_bnd == expected_cap_bnd, (
        f"CapBnd must match rendered CapabilityBoundingSet: "
        f"expected 0x{expected_cap_bnd:x}, got 0x{actual_cap_bnd:x}"
    )
    assert actual_cap_bnd != full_cap_mask, (
        f"CapBnd is the full kernel capability mask 0x{full_cap_mask:x}, not the bounded broker set"
    )
    assert actual_cap_eff & ~actual_cap_bnd == 0, (
        f"CapEff 0x{actual_cap_eff:x} contains bits outside CapBnd 0x{actual_cap_bnd:x}"
    )

    assert actual_cap_amb == expected_cap_amb, (
        f"CapAmb must match rendered AmbientCapabilities: "
        f"expected 0x{expected_cap_amb:x}, got 0x{actual_cap_amb:x}"
    )
    assert actual_cap_amb == 0, f"broker must not carry ambient capabilities, got 0x{actual_cap_amb:x}"

    assert actual_nonewprivs == expected_nonewprivs, (
        f"NoNewPrivs must match rendered NoNewPrivileges={unit['NoNewPrivileges']}, "
        f"got {actual_nonewprivs}"
    )
    assert actual_seccomp == 2, f"broker must run in seccomp filter mode (2), got {actual_seccomp}"

    assert any(expected_slice in path for path in cgroup_paths), (
        f"broker cgroup path must contain rendered Slice={expected_slice}, got {cgroup_paths}"
    )
    assert any("d2b.slice" in path for path in cgroup_paths), (
        f"broker cgroup path must contain d2b.slice, got {cgroup_paths}"
    )
  '';
}
