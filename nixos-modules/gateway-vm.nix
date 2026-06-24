# Auto-declare realm gateway guests from nixling.gateways.
{ config, lib, pkgs, ... }:

let
  cfg = config.nixling;
  gateways = lib.filterAttrs (_: gw: gw.enable) cfg.gateways;
  currentSystem = pkgs.stdenv.hostPlatform.system;

  unavailableHostToolPackage = name: pkgs.stdenvNoCC.mkDerivation {
    pname = name;
    version = "unavailable-for-${currentSystem}";
    dontUnpack = true;
    installPhase = ''
      echo "nixling gateway VM: ${name} is not available for ${currentSystem}; set nixling.site.usePrebuiltHostTools = false to use source host-tool packages." >&2
      exit 1
    '';
  };

  hostToolPackage = attr: name:
    let
      pkg = cfg._hostToolPackages.${attr} or null;
    in
    if pkg != null then pkg else unavailableHostToolPackage name;

  nixlingdPackage = hostToolPackage "nixlingd" "nixlingd";
  nixlingPackage = hostToolPackage "nixling" "nixling";
  nixlingGatewayRuntimePackage = hostToolPackage "nixlingGatewayRuntime" "nixling-gateway-runtime";

  secretShaped = s:
    lib.hasInfix "SharedAccessKey" s
    || lib.hasInfix "Endpoint=sb://" s
    || lib.hasInfix "AccountKey=" s
    || lib.hasInfix "PRIVATE KEY" s
    || lib.hasInfix "BEGIN " s;

  safeRuntimePath = s:
    lib.hasPrefix "/" s
    && lib.hasPrefix "${toString cfg.site.stateDir}/" s
    && !(builtins.elem ".." (lib.splitString "/" s))
    && !(lib.hasSuffix "/" s)
    && !(lib.hasPrefix "/nix/store/" s)
    && !(secretShaped s);

  gatewayStateDirs = gw:
    lib.unique (lib.filter safeRuntimePath [
      gw.stateDir
      (builtins.dirOf gw.credentialPath)
      (builtins.dirOf gw.sealKeyPath)
    ]);

  gatewayVm = name: gw: {
    name = gw.vmName;
    value = {
      enable = true;
      autostart = false;
      env = gw.env;
      index = gw.index;
      ssh.user = lib.mkDefault "gateway";
      config = { lib, pkgs, ... }: {
        networking.hostName = lib.mkDefault gw.vmName;
        users.groups.nixling = { };
        users.groups.nixlingd = { };
        users.users.nixlingd = {
          isSystemUser = true;
          group = "nixlingd";
          extraGroups = [ "nixling" ];
        };
        users.users.gateway = {
          isNormalUser = true;
          extraGroups = [ "wheel" "nixling" ];
        };
        environment.etc."nixling/daemon-config.json".text = builtins.toJSON {
          publicSocketPath = "/run/nixling/public.sock";
          brokerSocketPath = "/run/nixling/priv.sock";
          stateLockPath = "/run/nixling/daemon.lock";
          locksDir = "/run/nixling/locks";
          daemonUser = "nixlingd";
          daemonGroup = "nixlingd";
          publicSocketGroup = "nixling";
          launcherUsers = [ "gateway" ];
          adminUsers = [ "gateway" ];
          serverVersion = "0.4.0";
          acceptedClientVersionRange = ">=0.4.0, <0.5.0";
          gatewayConfigPath = "/etc/nixling/gateway.json";
          autostartParallelism = cfg.daemon.autostart.parallelism;
          gracefulShutdownTimeoutSeconds = cfg.daemon.lifecycle.gracefulShutdown.timeoutSeconds;
          artifacts = {
            publicManifestPath = "/etc/nixling/manifest.json";
            bundlePath = "/etc/nixling/bundle.json";
            hostPath = "/etc/nixling/host.json";
            processesPath = "/etc/nixling/processes.json";
            closuresDir = "/etc/nixling/closures";
          };
        };
        environment.etc."nixling/gateway.json".text = builtins.toJSON {
          gateway = name;
          realm = gw.realm;
          stateDir = gw.stateDir;
          credentialPath = gw.credentialPath;
          sealKeyPath = gw.sealKeyPath;
          inherit (gw) allowHostRelayCredentials;
          relay = {
            inherit (gw.relay) namespace entity;
          };
          aca = {
            inherit (gw.aca)
              endpoint
              subscription
              resourceGroup
              sandboxGroup
              region
              diskImageId
              image
              diskName
              managedIdentityResourceId
              managedIdentityClientId
              cpu
              memory
              autoSuspendIntervalSecs
              ;
          };
          display = {
            inherit (gw.display) vsockPort waypipeCompression;
          };
        };
        environment.etc."nixling/manifest.json".text = builtins.toJSON {
          _manifest = {
            manifestVersion = cfg._manifestVersion;
          };
          ${gw.vmName} = {
            name = gw.vmName;
            graphics = false;
            tpm = false;
            usbipYubikey = false;
            audio = false;
            tap = "${gw.env}-l${toString gw.index}";
            bridge = "br-${gw.env}-lan";
            env = gw.env;
            isNetVm = false;
            netVm = "sys-${gw.env}-net";
            usbipdHostIp = null;
            stateDir = "${cfg.site.stateDir}/vms/${gw.vmName}";
            apiSocket = "${cfg.site.stateDir}/vms/${gw.vmName}/${gw.vmName}.sock";
            gpuSocket = "${cfg.site.stateDir}/vms/${gw.vmName}/${gw.vmName}-gpu.sock";
            tpmSocket = "/run/nixling/vms/${gw.vmName}/tpm.sock";
            audioStateFile = "${cfg.site.stateDir}/vms/${gw.vmName}/state/audio-state.json";
            audioService = "nixling-${gw.vmName}-snd.service";
            observability = {
              enabled = false;
              vsockCid = 0;
              vsockHostSocket = "";
              agentSocket = "/run/nixling/otlp.sock";
            };
            staticIp = null;
            sshUser = "gateway";
          };
        };
        environment.variables.NIXLING_MANIFEST_PATH = "/etc/nixling/manifest.json";
        environment.systemPackages = with pkgs; [
          curl
          nixlingPackage
          nixlingdPackage
          nixlingGatewayRuntimePackage
        ];
        systemd.tmpfiles.rules = [
          "d /run/nixling 0750 nixlingd nixling -"
          "f /run/nixling/daemon.lock 0640 nixlingd nixlingd -"
          "d /run/nixling/locks 0700 nixlingd nixlingd -"
          "d /run/nixling/state 0700 nixlingd nixlingd -"
          "d /var/lib/nixling 0750 nixlingd nixlingd -"
          "d /var/lib/nixling/daemon-state 0700 nixlingd nixlingd -"
          "d /var/cache/nixling 0750 root nixlingd -"
        ] ++ (map (dir: "d ${dir} 0700 nixlingd nixlingd -") (gatewayStateDirs gw));
        systemd.services.nixlingd = {
          description = "nixling realm gateway daemon";
          wantedBy = [ "multi-user.target" ];
          after = [ "network-online.target" ];
          wants = [ "network-online.target" ];
          restartIfChanged = false;
          serviceConfig = {
            Type = "simple";
            User = "nixlingd";
            Group = "nixlingd";
            SupplementaryGroups = [ "nixling" ];
            ExecStart = "${nixlingdPackage}/bin/nixlingd serve --config /etc/nixling/daemon-config.json";
            Restart = "on-failure";
            RestartSec = "2s";
            NoNewPrivileges = true;
            CapabilityBoundingSet = [ "" ];
            AmbientCapabilities = [ "" ];
            PrivateTmp = true;
            ProtectHome = true;
            RestrictAddressFamilies = [ "AF_UNIX" "AF_INET" "AF_INET6" "AF_VSOCK" ];
            UMask = "0077";
          };
        };
      };
    };
  };
in
{
  nixling.vms = lib.mkMerge [
    (lib.listToAttrs (lib.mapAttrsToList gatewayVm gateways))
  ];
}
