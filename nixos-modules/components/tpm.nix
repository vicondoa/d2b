# TPM support for nixling VMs. Imported by host.nix whenever a VM
# sets `nixling.vms.<name>.tpm.enable = true`.
#
# Wires cloud-hypervisor's `--tpm socket=...` to the per-VM
# nixling-<name>-swtpm.service running on the host (see host.nix for
# the per-VM systemd unit). State persists in
# /var/lib/nixling/swtpm/<name>/ on the host across launches — wiping
# it looks like device tampering to remote IdPs and forces
# re-enrolment.
{ lib, pkgs, config, ... }:

{
  # cloud-hypervisor is the only hypervisor microvm.nix can talk to
  # swtpm with. mkDefault so graphics.nix (which also sets this)
  # doesn't conflict if both are enabled.
  microvm.hypervisor = lib.mkDefault "cloud-hypervisor";

  # cloud-hypervisor's --tpm path moved from /run/swtpm/<vm>/sock
  # to /run/nixling/vms/<vm>/tpm.sock. The per-VM runtime dir already exists,
  # is owned nixlingd:nixling 0750 with default ACL granting every
  # per-VM ephemeral UID rwx (see host-activation.nix
  # nixlingRoleUidAcls). Putting the TPM socket there lets cloud-hypervisor
  # connect via the inherited named-user ACL — no separate /run/swtpm/ dir
  # or per-VM ACL needed.
  microvm.cloud-hypervisor.extraArgs = [
      "--tpm" "socket=/run/nixling/vms/${config.networking.hostName}/tpm.sock"
  ];

  security.tpm2.enable = true;

  # The TPM CRB driver auto-probes when the kernel sees the cloud-
  # hypervisor TPM CRB device at the documented IO range
  # (fed40000-fed40fff). Explicit modules here are belt-and-suspenders
  # in case the auto-probe is suppressed by some other module init order.
  boot.kernelModules = [ "tpm" "tpm_crb" ];

  # In-guest TPM diagnostics. Useful before flipping any downstream
  # service (Himmelblau, sbctl, etc) that wants to bind keys:
  #   ls /dev/tpm*                            -> /dev/tpm0 /dev/tpmrm0
  #   sudo tpm2_getcap properties-fixed       -> swtpm manufacturer/firmware
  #   sudo tpm2_getrandom 16 | xxd            -> non-zero bytes
  environment.systemPackages = [ pkgs.tpm2-tools ];

  # Provision the TPM2 Storage Root Key (SRK) at the standard
  # persistent handle 0x81000001 before any service that wants to
  # bind keys tries to use it. ECC P-256 first (matches
  # systemd-tpm2-setup's algorithm preference), RSA-2048 fallback.
  # State persists in swtpm NVRAM so this runs at most once per VM.
  #
  # Anything that needs the SRK in place (e.g. himmelblaud) should
  # add `after = [ "tpm2-srk-provision.service" ]` in its own module.
  systemd.services.tpm2-srk-provision = {
    description = "Provision TPM2 SRK at 0x81000001";
    wantedBy = [ "multi-user.target" ];
    after = [ "systemd-modules-load.service" ];
    path = with pkgs; [ tpm2-tools coreutils ];
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
      User = "root";
    };
    environment = {
      # Force the direct device TCTI so tpm2-tools doesn't try to dial
      # tabrmd over D-Bus first (the warning it emits otherwise is
      # harmless but the attribute parser still chokes when the
      # tabrmd-style codepath leaves stderr noise interleaved with the
      # arg parser).
      TPM2TOOLS_TCTI = "device:/dev/tpmrm0";
    };
    script = ''
      set -e
      SRK_HANDLE=0x81000001
      if tpm2_getcap handles-persistent 2>/dev/null | grep -q "$SRK_HANDLE"; then
        echo "SRK already provisioned at $SRK_HANDLE; nothing to do."
        exit 0
      fi
      echo "Provisioning TPM2 SRK at $SRK_HANDLE..."
      CTX=$(mktemp /tmp/srk.XXXXXX.ctx)
      trap 'rm -f "$CTX"' EXIT
      # ECC P-256 first (systemd preference), RSA-2048 fallback.
      # Attributes match TCG TPM v2.0 Provisioning Guidance §7.5.1.
      # tpm2-tools 5.7's tpm2_attr_util.c only accepts LOWERCASE tokens
      # for the -a list — uppercase produces 'Unknown token: "DECRYPT"'.
      ATTRS="decrypt|fixedparent|fixedtpm|noda|restricted|sensitivedataorigin|userwithauth"
      if tpm2_createprimary -C o -G ecc256:aes128cfb \
          -a "$ATTRS" -c "$CTX"; then
        echo "Created ECC P-256 primary key."
      elif tpm2_createprimary -C o -G rsa2048:aes128cfb \
          -a "$ATTRS" -c "$CTX"; then
        echo "ECC not supported; created RSA-2048 primary key."
      else
        echo "ERROR: tpm2_createprimary failed for both ECC and RSA"
        exit 1
      fi
      tpm2_evictcontrol -C o -c "$CTX" "$SRK_HANDLE"
      echo "SRK persisted at $SRK_HANDLE."
    '';
  };
}
