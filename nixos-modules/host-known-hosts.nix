{ pkgs, lib, ... }:

let
  refreshScript = pkgs.writeShellScript "nixling-known-hosts-refresh" ''
    set -euo pipefail
    vm="''${1:?usage: nixling-known-hosts-refresh <vm>}"
    manifest=/run/current-system/sw/share/nixling/vms.json
    kh=/var/lib/nixling/known_hosts.nixling

    # Net VMs have no host-accessible sshd — skip silently.
    is_net_vm=$(${pkgs.jq}/bin/jq -r --arg vm "$vm" \
                  '.[$vm].isNetVm // false' "$manifest" 2>/dev/null || echo false)
    if [ "$is_net_vm" = "true" ]; then
      echo "nixling: known-hosts-refresh: $vm is a net VM — skipping"
      exit 0
    fi

    ip=$(${pkgs.jq}/bin/jq -r --arg vm "$vm" '.[$vm].staticIp // empty' \
          "$manifest" 2>/dev/null || true)
    if [ -z "$ip" ]; then
      echo "nixling: known-hosts-refresh: no staticIp for $vm — skipping"
      exit 0
    fi

    # v0.2.0+: sshd host keys are HOST-MANAGED (see
    # host-ssh-host-keys.nix). The authoritative pubkey lives at
    # ${"$"}{site.stateDir}/vms/<vm>/sshd-host-keys/ssh_host_ed25519_key.pub
    # and is shared into the guest read-only. Reading it directly is
    # both faster and immune to the live-vs-pinned drift the old
    # ssh-keyscan-based logic had to handle (a VM restart used to
    # regenerate the in-VM key every time).
    host_pubkey_file=/var/lib/nixling/vms/$vm/sshd-host-keys/ssh_host_ed25519_key.pub
    if [ ! -r "$host_pubkey_file" ]; then
      echo "nixling: known-hosts-refresh: host-side pubkey for $vm not present at $host_pubkey_file — skipping (will be generated on next activation)" >&2
      exit 0
    fi
    pubkey_body=$(${pkgs.coreutils}/bin/head -n1 "$host_pubkey_file" \
                  | ${pkgs.gawk}/bin/awk '{print $1 " " $2}')
    new_keys="$ip $pubkey_body"

    # Serialise the read/filter/write across concurrent refreshes
    # (multiple microvm@<vm>.service units can fire at once at boot).
    kh_lock=/var/lib/nixling/known_hosts.nixling.lock
    exec 9>"$kh_lock"
    ${pkgs.util-linux}/bin/flock -w 30 9 || {
      echo "nixling: known-hosts-refresh: could not acquire $kh_lock within 30s" >&2
      exit 1
    }

    # Idempotent: if the line already matches, no rewrite.
    if [ -f "$kh" ] && ${pkgs.gnugrep}/bin/grep -qx "$new_keys" "$kh"; then
      echo "nixling: known-hosts-refresh: $vm ($ip) host key unchanged — no-op"
      exit 0
    fi

    kh_dir=$(dirname "$kh")
    tmp=$(${pkgs.coreutils}/bin/mktemp "$kh_dir/.known_hosts_refresh.XXXXXX")
    trap '${pkgs.coreutils}/bin/rm -f "$tmp"' EXIT
    { [ -f "$kh" ] && ${pkgs.gnugrep}/bin/grep -v "^$ip " "$kh"; } \
      > "$tmp" 2>/dev/null || true
    printf '%s\n' "$new_keys" >> "$tmp"
    ${pkgs.coreutils}/bin/chmod 0644 "$tmp"
    ${pkgs.coreutils}/bin/chown root:root "$tmp"
    ${pkgs.coreutils}/bin/mv -f "$tmp" "$kh"
    echo "nixling: known-hosts-refresh: $kh updated for $vm ($ip) from host-managed pubkey"
  '';
in
{
  # ---------------------------------------------------------------------------
  # M2 — known_hosts TOFU-on-boot: refresh /var/lib/nixling/known_hosts.nixling
  # with the current host key each time a workload VM starts.
  #
  # The VM's sshd starts after microvm@ becomes active.  We wait up to 90s
  # for sshd to accept connections, then write the fresh ed25519 key into
  # known_hosts.nixling atomically (remove stale entry for this IP, append
  # new key).  Failures are non-fatal: a warning is logged and the old entry
  # is left in place so SSH still works if the key happened to be correct.
  #
  # Trigger: microvm@.service wants this template; specifier %i is the VM name.
  # Net VMs are detected from the manifest and skipped (no exposed sshd).
  # ---------------------------------------------------------------------------
  systemd.services."nixling-known-hosts-refresh@" = {
    description = "Refresh known_hosts.nixling for microVM %i (M2 TOFU-on-boot)";
    wantedBy = [ ];
    after = [ "microvm@%i.service" ];
    serviceConfig = {
      Type          = "oneshot";
      RemainAfterExit = false;
      User          = "root";
      ExecStart     = "${refreshScript} %i";
      StandardOutput = "journal";
      StandardError  = "journal";
    };
  };

  # Trigger the refresh whenever any VM (including net VMs) starts.
  # Net VMs are detected and skipped early in the refresh script itself.
  systemd.services."microvm@" = {
    wants = [ "nixling-known-hosts-refresh@%i.service" ];
    # v0.1.5: don't let NixOS override upstream microvm.nix's own
    # X-RestartIfChanged=false. Without this, any framework
    # config change cycles every running headless/net VM at
    # rebuild time (the framework adds the `wants` above, so
    # NixOS treats the unit as framework-owned and emits
    # X-RestartIfChanged=true in the drop-in). Same rationale as
    # the per-VM sidecars (host-sidecars.nix / audio host) —
    # consumer applies VM-closure changes via `nixling switch
    # <vm>`.
    restartIfChanged = false;
  };
}
