# Auto-declare realm gateway guests from d2b.gateways.
{ config, lib, pkgs, ... }:

let
  cfg = config.d2b;
  gateways = lib.filterAttrs (_: gw: gw.enable) cfg.gateways;
  currentSystem = pkgs.stdenv.hostPlatform.system;

  unavailableHostToolPackage = name: pkgs.stdenvNoCC.mkDerivation {
    pname = name;
    version = "unavailable-for-${currentSystem}";
    dontUnpack = true;
    installPhase = ''
      echo "d2b gateway VM: ${name} is not available for ${currentSystem}; set d2b.site.usePrebuiltHostTools = false to use source host-tool packages." >&2
      exit 1
    '';
  };

  hostToolPackage = attr: name:
    let
      pkg = cfg._hostToolPackages.${attr} or null;
    in
    if pkg != null then pkg else unavailableHostToolPackage name;

  d2bdPackage = hostToolPackage "d2bd" "d2bd";
  d2bPackage = hostToolPackage "d2b" "d2b";
  d2bGatewayRuntimePackage = hostToolPackage "d2bGatewayRuntime" "d2b-gateway-runtime";

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
        users.groups.d2b = { };
        users.groups.d2bd = { };
        users.users.d2bd = {
          isSystemUser = true;
          group = "d2bd";
          extraGroups = [ "d2b" ];
        };
        users.users.gateway = {
          isNormalUser = true;
          extraGroups = [ "wheel" "d2b" ];
        };
        environment.etc."d2b/daemon-config.json".text = builtins.toJSON {
          publicSocketPath = "/run/d2b/public.sock";
          brokerSocketPath = "/run/d2b/priv.sock";
          stateLockPath = "/run/d2b/daemon.lock";
          locksDir = "/run/d2b/locks";
          daemonUser = "d2bd";
          daemonGroup = "d2bd";
          publicSocketGroup = "d2b";
          launcherUsers = [ "gateway" ];
          adminUsers = [ "gateway" ];
          serverVersion = "0.4.0";
          acceptedClientVersionRange = ">=0.4.0, <0.5.0";
          gatewayConfigPath = "/etc/d2b/gateway.json";
          autostartParallelism = cfg.daemon.autostart.parallelism;
          gracefulShutdownTimeoutSeconds = cfg.daemon.lifecycle.gracefulShutdown.timeoutSeconds;
          liveActivationTimeoutSeconds = cfg.daemon.lifecycle.liveActivation.timeoutSeconds;
          artifacts = {
            publicManifestPath = "/etc/d2b/manifest.json";
            bundlePath = "/etc/d2b/bundle.json";
            hostPath = "/etc/d2b/host.json";
            processesPath = "/etc/d2b/processes.json";
            closuresDir = "/etc/d2b/closures";
          };
        };
        environment.etc."d2b/gateway.json".text = builtins.toJSON {
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
        environment.etc."d2b/manifest.json".text = builtins.toJSON {
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
            tpmSocket = "/run/d2b/vms/${gw.vmName}/tpm.sock";
            audioStateFile = "${cfg.site.stateDir}/vms/${gw.vmName}/state/audio-state.json";
            # audioService is always null: the per-VM `d2b-<vm>-snd.service`
            # systemd unit is retired. The audio sidecar runs as a
            # broker-spawned runner via SpawnRunner{role: Audio}.
            audioService = null;
            observability = {
              enabled = false;
              vsockCid = 0;
              vsockHostSocket = "";
              agentSocket = "/run/d2b/otlp.sock";
            };
            staticIp = null;
            sshUser = "gateway";
          };
        };
        environment.variables.D2B_MANIFEST_PATH = "/etc/d2b/manifest.json";
        environment.systemPackages = with pkgs; [
          curl
          d2bPackage
          d2bdPackage
          d2bGatewayRuntimePackage
        ];
        systemd.tmpfiles.rules = [
          "d /run/d2b 1770 root d2b -"
          "z /run/d2b 1770 root d2b -"
          "a+ /run/d2b - - - - g::r-x"
          "a+ /run/d2b - - - - u:d2bd:rwx"
          "a+ /run/d2b - - - - m::rwx"
          "f /run/d2b/daemon.lock 0640 d2bd d2bd -"
          "d /run/d2b/locks 0700 d2bd d2bd -"
          "d /run/d2b/state 0700 d2bd d2bd -"
          "d /var/lib/d2b 0750 d2bd d2bd -"
          "d /var/lib/d2b/daemon-state 0700 d2bd d2bd -"
          "d /var/cache/d2b 0750 root d2bd -"
        ] ++ (map (dir: "d ${dir} 0700 d2bd d2bd -") (gatewayStateDirs gw));
        systemd.services.d2bd = {
          description = "d2b realm gateway daemon";
          wantedBy = [ "multi-user.target" ];
          after = [ "network-online.target" ];
          wants = [ "network-online.target" ];
          restartIfChanged = false;
          serviceConfig = {
            Type = "simple";
            User = "d2bd";
            Group = "d2bd";
            SupplementaryGroups = [ "d2b" ];
            ExecStart = "${d2bdPackage}/bin/d2bd serve --config /etc/d2b/daemon-config.json";
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
  d2b.vms = lib.mkMerge [
    (lib.listToAttrs (lib.mapAttrsToList gatewayVm gateways))
  ];
}
