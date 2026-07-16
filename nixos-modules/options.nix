# d2b 2.0 public option schema.
{ config, lib, ... }:

{
  imports = [
    ./options-site.nix
    ./options-host.nix
    ./options-realms.nix
    ./options-daemon.nix
  ];

  options.d2b.acceptDestructiveV2Cutover = lib.mkOption {
    type = lib.types.bool;
    default = false;
    description = ''
      Acknowledge that enabling d2b 2.0 requires the destructive reset
      procedure and provides no d2b 1.x state, configuration, or protocol
      compatibility. This acknowledgement is an evaluation gate only; setting
      it does not delete state or initiate the reset procedure.
    '';
  };

  config.assertions = [
    {
      assertion = config.d2b.acceptDestructiveV2Cutover;
      message = ''
        d2b.acceptDestructiveV2Cutover must be set to true. d2b 2.0 has no
        compatibility path and its reset procedure destroys all d2b 1.x state,
        workload disks, TPM state, keys, credentials, audits, and sessions.
      '';
    }
  ];
}
