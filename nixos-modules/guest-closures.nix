# Private evaluated Guest closure metadata.
#
# A Guest system artifact is evaluated once by the consumer and then
# published as a broker-owned closure input.  The metadata is private:
# Resource specs continue to carry only the artifact ID, while StoreSync
# receives the realised closure registration and the Zone/Guest-qualified
# store-view location from the artifact catalog.
{ config, lib, pkgs, ... }:

let
  cfg = config.d2b;
  d2bLib = import ./lib.nix { inherit lib pkgs; };
  identity = import ./resources-bundle.nix { inherit lib; };

  guestRows = lib.concatMap
    (zoneName:
      let resources = (cfg.zones.${zoneName}.resources or { });
      in lib.mapAttrsToList
        (guestName: guest: {
          inherit zoneName guestName guest;
          spec = guest.spec or { };
          system = d2bLib.v3GuestSystemFor
            (cfg.guestSystems or { })
            zoneName
            guestName;
          artifact =
            let artifactId = (guest.spec or { }).systemArtifactId or null;
            in if builtins.isString artifactId
              && builtins.hasAttr artifactId (cfg.artifacts or { })
            then cfg.artifacts.${artifactId}
            else null;
        })
        (lib.filterAttrs
          (_: guest:
            guest.type == "Guest"
            && (guest.spec.providerRef or null)
              == "Provider/runtime-cloud-hypervisor")
          resources))
    (lib.sort lib.lessThan (lib.attrNames (cfg.zones or { })));

  evaluatorReady = guest:
    d2bLib.v3GuestEvaluatorReady guest.system
    && guest.artifact != null
    && (guest.artifact.type or null) == "nixos-system"
    && descriptorFor guest != null;

  descriptorFor = row:
    let
      compiler = cfg._resourceCompiler or { };
      projection = compiler.providerProjectionRuntimeCloudHypervisor or { };
      privateArtifact = projection.privateArtifact or { };
    in lib.findFirst
      (descriptor:
        descriptor.zone == row.zoneName
        && descriptor.guest == row.guestName)
      null
      (privateArtifact.guestSetupDescriptors or [ ]);

  providerExecutionRef = row:
    let
      resources = cfg.zones.${row.zoneName}.resources or { };
      provider =
        if builtins.hasAttr "runtime-cloud-hypervisor" resources
        then resources."runtime-cloud-hypervisor"
        else null;
      execution =
        if provider == null then null
        else (provider.spec.config or { }).controllerExecutionRef or null;
    in if execution == null then "Host/host-system" else execution;

  cloudHypervisorArgv = row: guestConfig: toplevel: stateDir:
    let
      microvm = guestConfig.microvm or { };
      vcpu = microvm.vcpu or 1;
      mem = microvm.mem or 512;
      vsock = microvm.vsock or { };
      vsockCid =
        if (vsock.cid or null) != null
        then vsock.cid
        else d2bLib.componentSessionVsockCid {
          name = "${row.zoneName}/${row.guestName}";
          index = null;
          envIndex = null;
        };
      vsockSocket = vsock.socket or "${stateDir}/vsock.sock";
      kernel = microvm.kernel or pkgs.linuxPackages.kernel;
      kernelPath =
        if pkgs.stdenv.hostPlatform.system == "x86_64-linux"
        then "${kernel.dev}/vmlinux"
        else "${kernel.out}/${pkgs.stdenv.hostPlatform.linux-kernel.target}";
      initrdPath = microvm.initrdPath or "${toplevel}/initrd";
      kernelParams =
        (microvm.kernelParams or [ ]) ++ [ "init=${toplevel}/init" ];
      shares = microvm.shares or [
        {
          source = "/nix/store";
          mountPoint = "/nix/.ro-store";
          tag = "ro-store";
          proto = "virtiofs";
        }
        {
          source = "${stateDir}/store-view/meta";
          mountPoint = "/run/d2b-store-meta";
          tag = "d2b-meta";
          proto = "virtiofs";
          readOnly = true;
        }
      ];
      shareArgs = lib.concatMap
        (share:
          lib.optional ((share.proto or "virtiofs") == "virtiofs") [
            "--fs"
            "socket=${stateDir}/${share.tag}.sock,tag=${share.tag}"
          ])
        shares;
      diskArgs =
        lib.optional (microvm.storeOnDisk or false) [
          "--disk"
          "path=${toString microvm.storeDisk},readonly=on"
        ]
        ++ lib.concatMap
          (volume: [
            "--disk"
            "path=${d2bLib.volumeHostPath stateDir row.guestName volume},serial=${d2bLib.volumeSerial volume}"
          ])
          (microvm.volumes or [ ]);
      netArgs = lib.concatMap
        (iface:
          if (iface.type or "tap") == "macvtap"
          then [ "--net" "fd=10,mac=${iface.mac}" ]
          else [ "--net" "tap=${iface.id},mac=${iface.mac}" ])
        (microvm.interfaces or [ ]);
    in [
      "microvm@${row.guestName}"
      "--cpus"
      "boot=${toString vcpu}"
      "--watchdog"
      "--kernel"
      kernelPath
      "--initramfs"
      (toString initrdPath)
      "--cmdline"
      (lib.concatStringsSep " " kernelParams)
      "--seccomp"
      "true"
      "--memory"
      "size=${toString mem}M,shared=on"
      "--console"
      "null"
      "--serial"
      "tty"
      "--vsock"
      "cid=${toString vsockCid},socket=${vsockSocket}"
      "--api-socket"
      "${stateDir}/${row.guestName}.sock"
    ] ++ lib.flatten diskArgs ++ lib.flatten shareArgs ++ netArgs
      ++ (microvm.cloud-hypervisor.extraArgs or [ ]);

  vmmFor = row: guestConfig: toplevel: stateDir:
    let
      descriptor = descriptorFor row;
      zoneUid = identity.stableUid "d2b:v3:zone-uid" row.zoneName;
      principal = "d2b-${row.zoneName}-${row.guestName}-runner";
      profileHash = builtins.hashString "sha256"
        "${row.zoneName}/${row.guestName}/cloud-hypervisor";
      binaryPackage =
        (guestConfig.microvm.cloud-hypervisor.package or pkgs.cloud-hypervisor);
    in {
      zoneUid = zoneUid;
      descriptorDigest =
        if descriptor == null then null else descriptor.descriptor.descriptorDigest;
      executionRef = providerExecutionRef row;
      binaryPath = "${binaryPackage}/bin/cloud-hypervisor";
      argv = cloudHypervisorArgv row guestConfig toplevel stateDir;
      env = [ "D2B_VM=${row.guestName}" ];
      stateDir = stateDir;
      deviceBinds = [ "/dev/kvm" "/dev/vhost-net" ];
      uid = d2bLib.stablePrincipalId principal;
      gid = d2bLib.stablePrincipalId principal;
      profileId = "ch-${builtins.substring 0 16 profileHash}";
      cgroupSubtree =
        "d2b.slice/${row.guestName}/cloud-hypervisor";
    };

  closureArtifact = row:
    let
      guestConfig = d2bLib.v3GuestConfigFor row.system;
      toplevel = guestConfig.system.build.toplevel;
      closure = pkgs.closureInfo { rootPaths = [ toplevel ]; };
      stateDir =
        "${toString cfg.site.stateDir}/zones/${row.zoneName}/guests/${row.guestName}";
      vmm = vmmFor row guestConfig toplevel stateDir;
      metadataPath = pkgs.runCommand
        "d2b-guest-closure-${row.zoneName}-${row.guestName}.json"
        {
          closureStorePaths = "${closure}/store-paths";
          closureRegistration = "${closure}/registration";
          inherit toplevel stateDir;
          artifactId = row.spec.systemArtifactId;
          zone = row.zoneName;
          guest = row.guestName;
          vmmJson = builtins.toJSON vmm;
          passAsFile = [
            "closureStorePaths"
            "closureRegistration"
            "toplevel"
            "stateDir"
            "artifactId"
            "zone"
            "guest"
            "vmmJson"
          ];
          nativeBuildInputs = [ pkgs.python3 ];
        }
        ''
          set -euo pipefail
          python3 - "$closureStorePathsPath" "$closureRegistrationPath" \
            "$toplevelPath" "$stateDirPath" "$artifactIdPath" \
            "$zonePath" "$guestPath" "$vmmJsonPath" "$out" <<'PY'
          import json
          import pathlib
          import sys

          (
              store_paths_path,
              registration_path,
              toplevel_path,
              state_dir_path,
              artifact_id_path,
              zone_path,
              guest_path,
              vmm_path,
              output_path,
          ) = sys.argv[1:]
          def read_arg(path):
              return pathlib.Path(path).read_text()

          store_paths = read_arg(store_paths_path)
          closure_paths = [
              line.strip()
              for line in pathlib.Path(store_paths).read_text().splitlines()
              if line.strip()
          ]
          registration = read_arg(registration_path)
          state_dir = read_arg(state_dir_path)
          vmm = json.loads(read_arg(vmm_path))
          payload = {
              "artifactId": read_arg(artifact_id_path),
              "closurePaths": closure_paths,
              "dbDumpPath": registration,
              "guest": read_arg(guest_path),
              "schemaVersion": "v3",
              "storeView": {
                  "mountPoint": "/nix/store",
                  "root": state_dir + "/store-view",
                  "sync": "broker-store-sync",
              },
              "toplevel": read_arg(toplevel_path),
              "zone": read_arg(zone_path),
              "vmm": vmm,
          }
          pathlib.Path(output_path).write_text(
              json.dumps(payload, sort_keys=True, separators=(",", ":")) + "\n",
              encoding="utf-8",
          )
          PY
        '';
      fixtureData = {
        schemaVersion = "v3";
        zone = row.zoneName;
        guest = row.guestName;
        artifactId = row.spec.systemArtifactId;
        toplevel = "${toplevel}";
        closurePaths = [ "${toplevel}" ];
        dbDumpPath = "${closure}/registration";
        storeView = {
          root = "${stateDir}/store-view";
          mountPoint = "/nix/store";
          sync = "broker-store-sync";
        };
        vmm = vmm;
      };
      metadata = builtins.fromJSON
        (builtins.unsafeDiscardStringContext (builtins.readFile metadataPath));
    in {
      data = metadata;
      fixtureData = metadata;
      path = metadataPath;
      installFileName = "closures/zones/${row.zoneName}/${row.guestName}.json";
      classification = "contractPrivateNonSecret";
      sensitivity = "nonSecret";
    };

  validRows = lib.filter evaluatorReady guestRows;
  artifacts = lib.listToAttrs (map
    (row:
      lib.nameValuePair
        "guestClosure-${row.zoneName}-${row.guestName}"
        (closureArtifact row))
    validRows);
in
{
  options.d2b._guestClosureArtifacts = lib.mkOption {
    type = lib.types.attrsOf lib.types.anything;
    default = { };
    internal = true;
    visible = false;
    description = "Private realised closure metadata for evaluated Zone Guests.";
  };

  config = {
    d2b._guestClosureArtifacts = artifacts;
    d2b._bundle.extraArtifacts = artifacts;
  };
}
