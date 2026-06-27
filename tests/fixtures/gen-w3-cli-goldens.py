#!/usr/bin/env python3
"""Generate W3 CLI golden table closure (W3fu1 H4).

For each row in the W3 closed CLI error-code table, emit paired:
  - tests/golden/cli-output/<command>-<code>.txt  (human envelope)
  - tests/golden/cli-output/<command>-<code>.json (JSON envelope)

Each envelope carries the 7 H4-mandated fields:
  kind / code / exit_code / what_was_checked / observed_state /
  remediation / docs_anchor

This script is shipped in-tree so the goldens can be regenerated when
the H4 table grows; the goldens themselves are checked in as the
authoritative spec, and `packages/d2b/tests/cli_contract_coverage.rs` enforces no
orphan rows.
"""
from __future__ import annotations

import json
import pathlib
import sys

OUT_DIR = pathlib.Path(__file__).resolve().parent.parent / "golden" / "cli-output"

ANCHOR = "docs/reference/error-codes.md"


def envelope(kind, code, exit_code, what_was_checked, observed_state, remediation):
    return {
        "kind": kind,
        "code": code,
        "exit_code": exit_code,
        "what_was_checked": what_was_checked,
        "observed_state": observed_state,
        "remediation": remediation,
        "docs_anchor": f"{ANCHOR}#{code}",
    }


# Closed table from plan.md §"W3 CLI contract docs + per-error golden
# coverage" §2691-2839. Each row maps to (kind, exit_code,
# what_was_checked, observed_state, remediation).
W3_ROWS = {
    # ---- host check (Tier-all, exit 1 unless noted) -------------------
    ("host-check", "cgroup-delegation-refused"): (
        "host-check-error", 1,
        "Whether systemd delegated +cpu +memory +io +pids to the d2b host slice.",
        "Delegation request refused; controllers missing from cgroup.subtree_control.",
        "Add `Delegate=cpu memory io pids` to /etc/systemd/system/d2b-host.slice and `systemctl daemon-reload`.",
    ),
    ("host-check", "cgroup-v2-unified-not-present"): (
        "host-check-error", 1,
        "Whether /sys/fs/cgroup is the unified v2 hierarchy.",
        "Hybrid or legacy v1 hierarchy detected at /sys/fs/cgroup.",
        "Boot the host with systemd.unified_cgroup_hierarchy=1 on the kernel command line.",
    ),
    ("host-check", "cgroup-controllers-missing"): (
        "host-check-error", 1,
        "Whether the host slice exposes the cpu/memory/io/pids controllers.",
        "Required controllers missing from /sys/fs/cgroup/d2b.slice/cgroup.controllers.",
        "Enable the missing controllers via systemd `Delegate=` on d2b-host.slice and reload.",
    ),
    ("host-check", "cgroup-kill-on-ancestor-refused"): (
        "host-check-error", 1,
        "Whether the broker can issue cgroup.kill on a leaf without escalating to an ancestor.",
        "cgroup.kill probe targeted an ancestor outside the d2b delegated subtree.",
        "Re-issue cgroup.kill against a leaf inside d2b.slice; never write cgroup.kill on an ancestor.",
    ),
    ("host-check", "ifname-too-long"): (
        "host-check-error", 78,
        "Whether every derived bridge/TAP ifname fits in IFNAMSIZ-1 (15 bytes).",
        "Derived ifname exceeds 15 bytes after hash truncation.",
        "Shorten the env/vm name in d2b.envs.<env>.name so the derived ifname stays <= 15 bytes.",
    ),
    ("host-check", "ifname-collision"): (
        "host-check-error", 78,
        "Whether two derived ifnames collide after hash truncation.",
        "Two distinct env/vm names produced the same d2b-* ifname suffix.",
        "Rename one of the colliding env/vm entries or pin a non-colliding `ifnameMapping` override.",
    ),
    ("host-check", "ipv6-sysctl-drift"): (
        "host-check-error", 1,
        "Whether net.ipv6.* sysctls match the host-prepare declared values.",
        "Observed sysctl value diverges from the host.json `ipv6Sysctls` declaration.",
        "Re-run `d2b host prepare --apply` so the broker re-asserts the declared sysctl set.",
    ),
    ("host-check", "nm-managed-foreign-conflict"): (
        "host-check-error", 1,
        "Whether a foreign NetworkManager profile is managing a d2b-owned interface.",
        "NetworkManager reports a managed connection on an interface declared as d2b-owned.",
        "Add the interface to /etc/NetworkManager/conf.d/d2b-unmanaged.conf and `nmcli general reload`.",
    ),
    ("host-check", "nm-reload-failed"): (
        "host-check-error", 1,
        "Whether `nmcli general reload conf` succeeded after writing the unmanaged drop-in.",
        "nmcli reload returned non-zero; D-Bus call to NetworkManager failed.",
        "Inspect `systemctl status NetworkManager` and retry; the broker will re-attempt on next apply.",
    ),
    ("host-check", "foreign-nft-rule-shadows-d2b"): (
        "host-check-error", 1,
        "Whether a foreign nft rule in a higher-priority hook shadows the inet d2b table.",
        "Foreign nft rule found at a hook priority lower than d2b's declared priority.",
        "Move the foreign rule to a higher priority or document it in d2b.firewallCoexistence.allow.",
    ),
    ("host-check", "firewall-coexistence-mismatch"): (
        "host-check-error", 1,
        "Whether the declared firewallCoexistence manager matches the running stack.",
        "host.firewallCoexistence.manager declared X; runtime probe observed Y.",
        "Update d2b.firewallCoexistence.manager to match the actual host firewall stack.",
    ),
    ("host-check", "host-modules-locked"): (
        "host-check-error", 1,
        "Whether kernel.modules_disabled=1 prevents loading required modules.",
        "/proc/sys/kernel/modules_disabled reads 1 but required modules are not preloaded.",
        "Either preload the required modules at boot or set modules_disabled=0 in the boot policy.",
    ),
    ("host-check", "modprobe-denied-not-in-matrix"): (
        "host-check-error", 1,
        "Whether every modprobe target is declared in the W3 kernel-module matrix.",
        "Broker observed a modprobe request for a module not present in modprobeAllowed.",
        "Add the module to d2b.kernelModules.allowed or remove its consumer.",
    ),
    ("host-check", "minijail-too-old"): (
        "host-check-error", 1,
        "Whether the nix-built minijail satisfies the W3 minimum version (v17).",
        "Detected minijail version older than 17; W3 sandbox profiles will not parse.",
        "Update to the Nix-built minijail v17+ pinned in pkgs/minijail.nix.",
    ),
    ("host-check", "ch-net-handoff-not-supported"): (
        "host-check-error", 1,
        "Whether the pinned Cloud Hypervisor exposes tap-fd net handoff.",
        "CH probe (--version + capability JSON) reports tap-fd net handoff is unsupported.",
        "Pin Cloud Hypervisor >= v40 or switch host.ch.netHandoffMode to `persistent-tap`.",
    ),
    ("host-check", "runner-shape-drift"): (
        "host-check-error", 1,
        "Whether the rendered runner argv matches the golden runner-shape snapshot.",
        "Runner argv diverges from tests/golden/runner-shape/* baseline.",
        "Regenerate the runner-shape baseline via the documented xtask or revert the drifted change.",
    ),
    ("host-check", "single-writer-conflict"): (
        "host-check-error", 78,
        "Whether legacy systemd units and the W3 daemon are both writing the host.",
        "Both `d2b-legacy.service` and `d2bd.service` are active on a Tier-0 mixed host.",
        "Stop the legacy units before starting the daemon; consult docs/explanation/host-prepare.md#mixed-tier-0.",
    ),
    ("host-check", "tier-0-legacy-uses-nixos-module"): (
        "host-check-error", 78,
        "Whether a Tier-0 all-legacy host is configured via the W3 daemon path.",
        "host.tier == 0 / legacy but d2bd.service was reached.",
        "Use the NixOS module on Tier-0 all-legacy hosts; the W3 daemon path is for Tier 1+.",
    ),
    ("host-check", "host-lan-cidr-ambiguous"): (
        "host-check-error", 1,
        "Whether the host LAN CIDR can be uniquely inferred from the live routing table.",
        "Multiple plausible LAN CIDRs detected (multi-iface + VPN); auto-inference refused.",
        "Set d2b.site.hostLanCidr explicitly in the bundle.",
    ),
    # ---- host prepare --apply -----------------------------------------
    ("host-prepare", "cgroup-delegation-refused"): (
        "host-prepare-apply-error", 1,
        "Whether the broker delegated +cpu +memory +io +pids before launching VMs.",
        "Apply phase observed the same delegation failure as the read-only probe.",
        "See remediation for host-check#cgroup-delegation-refused; re-run prepare after the fix.",
    ),
    ("host-prepare", "route-preflight-no-default-route"): (
        "host-prepare-apply-error", 1,
        "Whether the host has a default route before d2b adds NAT routes.",
        "ip route show default returned empty; d2b refuses to add routes without an upstream.",
        "Configure a default route on the host upstream before running `host prepare --apply`.",
    ),
    ("host-prepare", "route-preflight-foreign-default-route"): (
        "host-prepare-apply-error", 1,
        "Whether the host's default route is on a d2b-owned interface.",
        "Default route observed via a d2b-owned bridge; d2b refuses to claim it.",
        "Move the default route to the upstream uplink before running `host prepare --apply`.",
    ),
    ("host-prepare", "dnsmasq-not-bound"): (
        "host-prepare-apply-error", 1,
        "Whether the per-env dnsmasq is bound to its bridge after reconcile.",
        "ss -lnp probe shows no dnsmasq listening on the d2b-* bridge IP.",
        "Inspect systemctl status d2b-dnsmasq@<env>.service and retry `host prepare --apply`.",
    ),
    ("host-prepare", "path-safety-violation"): (
        "host-prepare-apply-error", 1,
        "Whether broker fs writes were performed via openat2+O_NOFOLLOW+RESOLVE_BENEATH.",
        "Target path observed to be a symlink, world-writable parent, or non-root parent.",
        "Inspect the path called out in the error; remove the symlink/hostile mount and retry.",
    ),
    ("host-prepare", "nm-reload-failed"): (
        "host-prepare-apply-error", 1,
        "Whether nmcli reload conf succeeded after broker rewrote the unmanaged drop-in.",
        "Apply phase observed nmcli reload returned non-zero.",
        "Inspect NetworkManager logs and retry `host prepare --apply`.",
    ),
    ("host-prepare", "bridge-port-flag-drift"): (
        "host-prepare-apply-error", 1,
        "Whether bridge port flags (learning/flooding/etc.) match the host.json declaration.",
        "Netlink readback shows a port flag diverging from bridgePortFlags.",
        "Re-run `host prepare --apply` or update host.bridgePortFlags to match the desired state.",
    ),
    ("host-prepare", "nft-foreign-rule-flush-attempted"): (
        "host-prepare-apply-error", 1,
        "Whether nft apply refused to flush a foreign table.",
        "Broker observed an nft ruleset replace targeting a non-d2b table; refused.",
        "Restrict nft batches to `table inet d2b`; never `flush ruleset` from d2b.",
    ),
    ("host-prepare", "firewall-coexistence-mismatch"): (
        "host-prepare-apply-error", 1,
        "Whether the declared firewallCoexistence manager matches the running stack at apply time.",
        "Apply phase observed the same firewallCoexistence drift as the read-only probe.",
        "See remediation for host-check#firewall-coexistence-mismatch; re-run apply after the fix.",
    ),
    ("host-prepare", "tier-0-legacy-uses-nixos-module"): (
        "host-prepare-apply-error", 78,
        "Whether a Tier-0 all-legacy host is invoking the W3 apply path.",
        "host.tier == 0 / legacy but d2bd.service was asked to mutate the host.",
        "Use the NixOS module on Tier-0 all-legacy hosts; the W3 daemon path is for Tier 1+.",
    ),
    ("host-prepare", "single-writer-conflict"): (
        "host-prepare-apply-error", 78,
        "Whether legacy + daemon writers are both targeting host state at apply time.",
        "Both legacy units and d2bd.service hold an exclusive writer claim.",
        "Stop legacy units before running `host prepare --apply` on a Tier-0 mixed host.",
    ),
    ("host-prepare", "legacy-no-prepare-apply"): (
        "host-prepare-apply-error", 78,
        "Whether the legacy bash dispatch attempted a mutating prepare apply.",
        "Legacy bash `d2b host prepare --apply` invoked; refused by design.",
        "Re-run via the Rust path: `d2b host prepare --apply` (no legacy shim).",
    ),
    # ---- host destroy --apply -----------------------------------------
    ("host-destroy", "vm-still-running-refused"): (
        "host-destroy-apply-error", 1,
        "Whether all VMs in the bundle are stopped before `host destroy --apply`.",
        "At least one VM is still running (cloud-hypervisor process detected).",
        "Stop the listed VMs with `d2b down <vm>` then retry `host destroy --apply`.",
    ),
    ("host-destroy", "tier-0-legacy-uses-nixos-module"): (
        "host-destroy-apply-error", 78,
        "Whether a Tier-0 all-legacy host is invoking the W3 destroy path.",
        "host.tier == 0 / legacy but d2bd.service was asked to tear the host down.",
        "Use the NixOS module path on Tier-0 all-legacy hosts.",
    ),
    ("host-destroy", "legacy-no-destroy-apply"): (
        "host-destroy-apply-error", 78,
        "Whether the legacy bash dispatch attempted a mutating destroy apply.",
        "Legacy bash `d2b host destroy --apply` invoked; refused by design.",
        "Re-run via the Rust path: `d2b host destroy --apply` (no legacy shim).",
    ),
    # ---- host install -------------------------------------------------
    ("host-install", "not-yet-implemented"): (
        "host-install-error", 70,
        "Whether `host install` is implemented in this d2b release.",
        "host install is a W4 deliverable; W3 ships the schema + CLI surface only.",
        "Use `d2b switch` or the NixOS module integration path until W4 ships.",
    ),
    # ---- Inherited W3-relevant onboarding rows (host check) -----------
    ("host-check", "daemon-down"): (
        "host-check-error", 1,
        "Whether d2bd.service is active on the host.",
        "systemctl is-active d2bd reports inactive/failed.",
        "Start the daemon: `systemctl start d2bd.service`.",
    ),
    ("host-check", "socket-perms-wrong"): (
        "host-check-error", 1,
        "Whether /run/d2b/public.sock has the expected mode/owner/group.",
        "Observed mode/owner/group diverges from the declared SocketSpec.",
        "Inspect `systemctl status d2bd.socket`; the unit re-asserts perms on restart.",
    ),
    ("host-check", "missing-group"): (
        "host-check-error", 1,
        "Whether the `d2b` group exists on the host.",
        "getent group d2b returned nothing.",
        "Create the group via the NixOS module (it manages users.groups.d2b).",
    ),
    ("host-check", "unsupported-kernel"): (
        "host-check-error", 1,
        "Whether the running kernel is >= 6.6 (W3 minimum).",
        "uname -r reports a kernel older than 6.6.",
        "Upgrade to a kernel >= 6.6 (Ubuntu 24.04 ships 6.8; see tests/golden/l3-matrix/w3-ubuntu.txt).",
    ),
    ("host-check", "no-kvm"): (
        "host-check-error", 1,
        "Whether /dev/kvm is present and accessible.",
        "/dev/kvm missing or not accessible to the d2b group.",
        "Load the appropriate kvm_intel/kvm_amd module and ensure the d2b group has access.",
    ),
    ("host-check", "no-cgroup-v2"): (
        "host-check-error", 1,
        "Whether the host runs the unified cgroup v2 hierarchy.",
        "/sys/fs/cgroup/cgroup.controllers missing or hybrid layout detected.",
        "Boot with systemd.unified_cgroup_hierarchy=1.",
    ),
    ("host-check", "nftables-conflict"): (
        "host-check-error", 1,
        "Whether a conflicting nft framework (firewalld/ufw raw) is active.",
        "Probe detected an unmanaged nft framework conflicting with d2b's policy.",
        "Configure firewallCoexistence to declare the running manager, or stop the conflicting service.",
    ),
    ("host-check", "hardlink-fs-mismatch"): (
        "host-check-error", 1,
        "Whether the per-VM /nix/store hardlink farm filesystem supports hardlinks.",
        "The store and the per-VM farm are on different filesystems; hardlinks impossible.",
        "Co-locate the per-VM store-view path on the same filesystem as /nix/store.",
    ),
    ("host-check", "manifest-skew"): (
        "host-check-error", 1,
        "Whether the installed manifest matches the activated NixOS generation.",
        "The bundleVersion in vms.json does not match the activated generation.",
        "Re-run `nixos-rebuild switch` to align the activated generation with the manifest.",
    ),
    ("host-check", "profile-rejects-root"): (
        "host-check-error", 1,
        "Whether the minijail profile correctly refuses uid 0 inside the sandbox.",
        "Minijail dry-run permitted uid 0 to retain capabilities; profile is too lax.",
        "Tighten the minijail profile in nixos-modules/components/sandbox.nix.",
    ),
    ("host-check", "seccomp-denial"): (
        "host-check-error", 1,
        "Whether the seccomp policy denies undeclared syscalls.",
        "Probe issued an undeclared syscall and the policy allowed it.",
        "Tighten the seccomp policy to match the declared syscall matrix.",
    ),
    ("host-check", "tap-creation-denied"): (
        "host-check-error", 1,
        "Whether the broker can create a TAP fd via TUNSETIFF.",
        "TUNSETIFF returned EPERM; CAP_NET_ADMIN missing from the broker capability set.",
        "Ensure d2b-priv-broker.service carries CAP_NET_ADMIN.",
    ),
    ("host-check", "stale-lock"): (
        "host-check-error", 1,
        "Whether /run/d2b/locks/<vm> is owned by a live process.",
        "Lockfile exists but the recorded PID is not running.",
        "Remove the stale lock via the documented recovery runbook; do not unlink by hand.",
    ),
}


def humanize(env: dict) -> str:
    return (
        f"error: {env['kind']}\n"
        f"  code: {env['code']}\n"
        f"  exit_code: {env['exit_code']}\n"
        f"  what_was_checked: {env['what_was_checked']}\n"
        f"  observed_state: {env['observed_state']}\n"
        f"  remediation: {env['remediation']}\n"
        f"  docs_anchor: {env['docs_anchor']}\n"
    )


def main():
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    count = 0
    for (cmd, code), (kind, exit_code, checked, observed, remediation) in W3_ROWS.items():
        env = envelope(kind, code, exit_code, checked, observed, remediation)
        stem = f"{cmd}-{code}"
        (OUT_DIR / f"{stem}.json").write_text(
            json.dumps(env, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
        (OUT_DIR / f"{stem}.txt").write_text(humanize(env), encoding="utf-8")
        count += 1
    print(f"wrote {count} golden pairs to {OUT_DIR}", file=sys.stderr)


if __name__ == "__main__":
    main()
