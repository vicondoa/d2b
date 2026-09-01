# Guest-side NixOS configuration for the `work-entra` Guest.
#
# The host declares `Guest/work-entra` in its Zone resource graph. This file
# remains a consumer-owned guest evaluator module; it is not a host lifecycle
# or Provider configuration file.
{ ... }:

{
  networking.hostName = "work-entra";

  users.users.alice = {
    isNormalUser = true;
    uid = 1000;
    extraGroups = [ "wheel" ];
  };

  entrablau = {
    enable = true;
    domain = [ "contoso.com" ];
    userMap.alice = "alice@contoso.com";
    joinType = "join";
    localUser = "alice";
    intuneCompliance = {
      enable = true;
      dmiOverride = {
        sys_vendor = "Example Corp";
        product_name = "Example Workstation";
        board_vendor = "Example Corp";
        board_name = "EX-WS-15";
      };
    };
  };

  system.stateVersion = "25.11";
}
