# Guest-side NixOS configuration for the `work-vm` Entra workspace.
# This module is merged into the VM's `config.imports` from
# `flake.nix` alongside `nixos-entra-id.nixosModules.default`, so
# every option here is set INSIDE the VM, not on the host.
#
# The split of responsibility is:
#
#   nixling.vms.work-vm.tpm.enable = true   (in flake.nix)
#     -> wires swtpm on the host, exposes /dev/tpmrm0 in the guest
#
#   nixosEntraId.* (here)
#     -> Himmelblau daemon, PAM/NSS, Intune compliance shimming
#
# Both halves are needed: Entra Conditional Access requires a
# hardware-rooted device identity (TPM 2.0), and Himmelblau is the
# Linux-native client that speaks the Entra protocols. Without TPM,
# Himmelblau falls back to software-bound keys, which most CA
# policies refuse.
{ lib, ... }:

{
  networking.hostName = "work-vm";

  # Required so /dev/tpmrm0 is reachable + the `tss` group exists
  # for himmelblaud's DynamicUser. The nixling TPM component sets
  # this too (via `nixos-modules/components/tpm.nix`), so this is
  # belt-and-suspenders; explicit here for readability when this
  # file is read on its own.
  security.tpm2.enable = true;

  # In-guest user. Matches `nixling.vms.work-vm.ssh.user` from
  # flake.nix so the framework's authorized-key injection lands
  # in the right ~/.ssh/authorized_keys.
  users.users.alice = {
    isNormalUser = true;
    uid = 1000;
    extraGroups = [ "wheel" ];
  };

  # The Entra-ID configuration. Everything under `nixosEntraId.*`
  # is owned by the sibling `vicondoa/nixos-entra-id` flake — see
  # its README for the full schema and per-option semantics. The
  # generic placeholders below MUST be replaced with your tenant
  # values before this VM will actually authenticate against
  # Entra.
  nixosEntraId = {
    enable = true;

    # TODO: replace with your tenant domain.
    domain = [ "contoso.com" ];

    # TODO: map your local-user → UPN. The local-user key must
    # match `users.users.<name>` above; the value is the Entra
    # UPN you sign in with.
    userMap.alice = "alice@contoso.com";

    # `join` = Azure-AD-Joined, Intune-enrolled (corporate-managed).
    # `register` = Azure-AD-Registered, BYOD (no Intune). Most
    # corporate Conditional Access policies require `join`.
    joinType = "join";

    localUser = "alice";

    intuneCompliance = {
      # Set to false on BYOD / Azure-AD-Registered deployments
      # that aren't enrolled in Intune. With `joinType = "join"`
      # above, leave this enabled.
      enable = true;

      # DMI / SMBIOS values bind-mounted into the himmelblau
      # service mount namespaces only — defeats Intune flagging a
      # virtualised guest as `Cloud Hypervisor` / `KVM`.
      #
      # GENERIC PLACEHOLDERS. In a real deployment you crib these
      # from a `dmidecode -t system,baseboard` dump on a real
      # device of a supported make/model. See the nixos-entra-id
      # README for guidance on what counts as "supported" and the
      # Intune compliance disclaimer below in this example's
      # README for the rules of the road.
      fakeDmi = {
        sys_vendor = "Example Corp";
        product_name = "Example Workstation";
        board_vendor = "Example Corp";
        board_name = "EX-WS-15";
      };
    };
  };

  system.stateVersion = "25.11";
}
