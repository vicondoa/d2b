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

    # security-r8-audio-13: retry up to 18 times (90s total)
    # waiting for the guest sshd to start. Heavy workload VMs
    # (virtio-gpu + audio + Wayland) need more time to boot
    # than headless net VMs; the previous 30s budget timed out
    # before sshd was reachable, leaving the host without any
    # pinned key for the VM and the launcher then fails its
    # SSH-ready probe forever. 90s comfortably covers the
    # slowest observed boots (~70s for a cold-cache rebuild
    # plus virtiofsd warm-up).
    new_keys=""
    for i in $(${pkgs.coreutils}/bin/seq 1 18); do
      sleep 5
      new_keys=$(${pkgs.openssh}/bin/ssh-keyscan \
                   -t ed25519,rsa -T 5 "$ip" 2>/dev/null) || true
      [ -n "$new_keys" ] && break
      echo "nixling: known-hosts-refresh: attempt $i/18 — no keys from $vm ($ip)"
    done

    if [ -z "$new_keys" ]; then
      echo "nixling: known-hosts-refresh: all attempts failed for $vm ($ip) — known_hosts not updated"
      exit 0
    fi

    # P7r2 security-r7-2: serialize the entire compare+rewrite block
    # with flock on the SAME lock file the CLI do_trust path uses
    # (/var/lib/nixling/known_hosts.nixling.lock). Multiple
    # nixling-known-hosts-refresh@<vm>.service instances can fire
    # concurrently at boot (e.g. when several microvm@<vm> units start
    # at once); without flock, the read/filter/write/mv pattern can
    # drop another VM's freshly pinned entry. The lock file is
    # pre-created by systemd.tmpfiles above as root:nixling-launcher 0660.
    kh_lock=/var/lib/nixling/known_hosts.nixling.lock
    exec 9>"$kh_lock"
    ${pkgs.util-linux}/bin/flock -w 30 9 || {
      echo "nixling: known-hosts-refresh: could not acquire $kh_lock within 30s" >&2
      exit 1
    }

    # P7r1 security-r7-1: M2 host-key pinning means refresh MUST NOT
    # silently overwrite an existing pinned key. Three branches:
    #
    #  1. No existing entry for $ip → TOFU-trust (write new keys; first-use).
    #  2. Existing entries match the new key set exactly → no-op (idempotent).
    #  3. Existing entries DIFFER from new keys → handled by the
    #     auto-rotation logic below (security-r8-audio-12).
    new_normalized=$(printf '%s\n' "$new_keys" | ${pkgs.gnused}/bin/sed 's/[[:space:]]*$//' | sort)
    existing_normalized=""
    if [ -f "$kh" ]; then
      existing_normalized=$(${pkgs.gnugrep}/bin/grep -E "^$ip " "$kh" 2>/dev/null \
        | ${pkgs.gnused}/bin/sed 's/[[:space:]]*$//' | sort || true)
    fi

    gen_pin_file=/var/lib/nixling/vms/$vm/known-host-key-generation
    current_gen=""
    if cg=$(${pkgs.coreutils}/bin/readlink "/var/lib/nixling/vms/$vm/store-meta/current" 2>/dev/null); then
      current_gen=$(${pkgs.coreutils}/bin/basename "$cg")
    fi

    if [ -z "$existing_normalized" ]; then
      echo "nixling: known-hosts-refresh: TOFU-pinning new host keys for $vm ($ip)"
    elif [ "$existing_normalized" = "$new_normalized" ]; then
      [ -n "$current_gen" ] && \
        printf '%s\n' "$current_gen" > "$gen_pin_file" 2>/dev/null || true
      ${pkgs.coreutils}/bin/chmod 0644 "$gen_pin_file" 2>/dev/null || true
      echo "nixling: known-hosts-refresh: $vm ($ip) host keys unchanged — no-op"
      exit 0
    else
      pinned_gen=""
      if [ -r "$gen_pin_file" ]; then
        pinned_gen=$(${pkgs.coreutils}/bin/head -n1 "$gen_pin_file" 2>/dev/null \
          | ${pkgs.coreutils}/bin/tr -d '[:space:]')
      fi
      if [ -n "$current_gen" ] && [ -n "$pinned_gen" ] \
         && [ "$current_gen" != "$pinned_gen" ]; then
        echo "nixling: known-hosts-refresh: ROTATING pinned key for $vm ($ip): VM generation $pinned_gen → $current_gen (legitimate rebuild)"
      elif [ -z "$pinned_gen" ] && [ -n "$current_gen" ]; then
        echo "nixling: known-hosts-refresh: no generation sidecar for $vm yet; adopting current gen=$current_gen and rotating once"
      else
        echo "nixling: known-hosts-refresh: REFUSING to overwrite pinned key for $vm ($ip): same generation $current_gen" >&2
        echo "nixling: known-hosts-refresh: live key differs from the pinned entry in $kh but the VM has NOT been rebuilt." >&2
        echo "nixling: known-hosts-refresh: treat as a possible MITM/host-swap incident." >&2
        echo "nixling: known-hosts-refresh: to force-trust anyway, run:" >&2
        echo "          sudo ssh-keygen -R $ip -f $kh && sudo systemctl start nixling-known-hosts-refresh@$vm.service" >&2
        exit 1
      fi
    fi

    # Atomic update: remove stale entries for this IP, append fresh keys.
    kh_dir=$(dirname "$kh")
    tmp=$(${pkgs.coreutils}/bin/mktemp "$kh_dir/.known_hosts_refresh.XXXXXX")
    trap '${pkgs.coreutils}/bin/rm -f "$tmp"' EXIT
    { [ -f "$kh" ] && ${pkgs.gnugrep}/bin/grep -v "^$ip " "$kh"; } \
      > "$tmp" 2>/dev/null || true
    printf '%s\n' "$new_keys" >> "$tmp"
    ${pkgs.coreutils}/bin/chmod 0644 "$tmp"
    ${pkgs.coreutils}/bin/chown root:root "$tmp"
    ${pkgs.coreutils}/bin/mv -f "$tmp" "$kh"
    if [ -n "$current_gen" ]; then
      printf '%s\n' "$current_gen" > "$gen_pin_file"
      ${pkgs.coreutils}/bin/chmod 0644 "$gen_pin_file"
    fi
    echo "nixling: known-hosts-refresh: $kh updated for $vm ($ip) at gen=$current_gen"
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
  };
}
