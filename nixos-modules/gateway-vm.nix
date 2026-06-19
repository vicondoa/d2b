# Auto-declare realm gateway guests from nixling.gateways.
{ config, lib, pkgs, ... }:

let
  cfg = config.nixling;
  gateways = lib.filterAttrs (_: gw: gw.enable) cfg.gateways;
  packagesSrc = lib.cleanSourceWith {
    src = ../packages;
    filter = path: type:
      let
        rel = lib.removePrefix (toString ../packages + "/") (toString path);
        parts = lib.splitString "/" rel;
      in !(builtins.elem "target" parts || lib.hasPrefix ".cargo/registry/" rel || lib.hasInfix "/.cargo/registry/" rel);
  };
  cargoLock = {
    lockFile = ../packages/Cargo.lock;
    outputHashes."wl-proxy-0.1.2" = "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=";
  };
  buildRustBin = packageName: binName: pkgs.rustPlatform.buildRustPackage {
    pname = binName;
    version = "0.0.0-bootstrap";
    src = packagesSrc;
    inherit cargoLock;
    cargoBuildFlags = [ "--package" packageName "--bin" binName ];
    doCheck = false;
    postPatch = ''
      mkdir -p .cargo
      cat > .cargo/config.toml <<EOF
      [build]
      rustc-wrapper = ""
      EOF
      rm -f .cargo/rustc-wrapper.sh
    '';
    installPhase = ''
      runHook preInstall
      install -Dm755 target/x86_64-unknown-linux-gnu/release/${binName} $out/bin/${binName} 2>/dev/null \
        || install -Dm755 target/release/${binName} $out/bin/${binName}
      runHook postInstall
    '';
  };
  nixlingdPackage = buildRustBin "nixlingd" "nixlingd";
  nixlingPackage = buildRustBin "nixling" "nixling";
  gatewayRelayPackage = buildRustBin "nixling-gateway-runtime" "nixling-gateway-relay";

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
          extraGroups = [ "wheel" ];
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
          gatewayRelayPackage
        ];
        systemd.tmpfiles.rules = [
          "d /run/nixling 0750 nixlingd nixling -"
          "d /run/nixling/locks 0750 nixlingd nixling -"
          "d ${gw.stateDir} 0700 gateway gateway -"
        ];
        systemd.services.nixlingd = {
          description = "nixling realm gateway daemon";
          wantedBy = [ "multi-user.target" ];
          after = [ "network-online.target" ];
          wants = [ "network-online.target" ];
          serviceConfig = {
            ExecStart = "${nixlingdPackage}/bin/nixlingd serve --config /etc/nixling/daemon-config.json --no-drop-privileges";
            Restart = "on-failure";
            User = "root";
            Group = "root";
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
