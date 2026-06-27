# Host ifname pure-eval checks retained alongside the rendered-artifact Rust
# contract test: the smoke bundle must emit at least one ifNameMappings row,
# every derivedIfname must keep the exact `^d2b-[bt][0-9A-F]{8}$` shape, and
# bridge/TAP rows must carry the role tag that the Rust predicate accepts.
{ mkEval, lib, flakeRoot, ... }:

let
  smokeConfig = { lib, ... }: {
    boot.loader.grub.enable = false;
    boot.loader.systemd-boot.enable = false;
    boot.initrd.includeDefaultModules = false;
    fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
    environment.etc."machine-id".text =
      "00000000000000000000000000000000";
    system.stateVersion = "25.11";
    users.users.alice = { isNormalUser = true; uid = 1000; };
    d2b.site = {
      waylandUser = "alice";
      launcherUsers = [ "alice" ];
      yubikey.enable = false;
    };
    d2b.envs.work = {
      lanSubnet = "10.20.0.0/24";
      uplinkSubnet = "192.0.2.0/30";
    };
    d2b.vms.corp-vm = {
      enable = true;
      env = "work";
      index = 10;
      ssh.user = "alice";
      config = {
        networking.hostName = lib.mkDefault "corp-vm";
        users.users.alice = { isNormalUser = true; uid = 1000; };
      };
    };
  };

  cfg = (mkEval [ smokeConfig ]).config;
  hostJson = builtins.fromJSON cfg.d2b._bundle.hostJson.jsonText;
  mappings = hostJson.ifNameMappings or [ ];
  regex = "^d2b-[bt][0-9A-F]{8}$";
  matchesShape = name: builtins.match regex name != null;
  tagFor = row: builtins.substring 4 1 row.derivedIfname;
  expectedRoleTags = [
    { role = "net-vm-lan"; tag = "b"; }
    { role = "uplink"; tag = "b"; }
    { role = "workload-lan"; tag = "t"; }
  ];
  roleTagPresent = wanted:
    lib.any
      (row: row.role == wanted.role && tagFor row == wanted.tag)
      mappings;
  hostJsonSource = builtins.readFile (flakeRoot + "/nixos-modules/host-json.nix");
in
{
  "ifname-rendered-host-json/ifname-mappings-non-empty" = {
    expr = mappings != [ ];
    expected = true;
  };

  "ifname-rendered-host-json/derived-ifnames-match-regex" = {
    expr = map
      (row: {
        inherit (row) role userVisibleName derivedIfname;
        matches = matchesShape row.derivedIfname;
      })
      mappings;
    expected = map
      (row: {
        inherit (row) role userVisibleName derivedIfname;
        matches = true;
      })
      mappings;
  };

  "ifname-rendered-host-json/bridge-and-tap-role-tags" = {
    expr = map
      (wanted: wanted // { present = roleTagPresent wanted; })
      expectedRoleTags;
    expected = map
      (wanted: wanted // { present = true; })
      expectedRoleTags;
  };

  "ifname-rendered-host-json/host-runtime-override-source-hook-present" = {
    expr =
      lib.hasInfix ''builtins.getEnv "D2B_HOST_RUNTIME_PATH"'' hostJsonSource
      && lib.hasInfix "runtimeRow.derivedIfname" hostJsonSource;
    expected = true;
  };
}
