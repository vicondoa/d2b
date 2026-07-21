{ system ? builtins.currentSystem
, pkgs ? import <nixpkgs> { inherit system; }
, flake ? builtins.getFlake ("git+file://" + toString ./../../../..)
}:

let
  inherit (pkgs) lib;
  nixos = flake.inputs.nixpkgs.lib.nixosSystem {
    inherit system;
    modules = [
      flake.nixosModules.default
      ({ lib, ... }: {
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
        d2b.acceptDestructiveV2Cutover = true;
        d2b.realms.work = {
          path = "work";
          placement = "host-local";
          broker = {
            enable = true;
            hostMutation = true;
          };
          network = {
            mode = "declared";
            lanSubnet = "10.20.0.0/24";
            uplinkSubnet = "192.0.2.0/30";
          };
          providers.runtime = {
            type = "runtime";
            implementationId = "cloud-hypervisor";
          };
          workloads.corp = {
            providerRefs.runtime = "runtime";
            config = {
              d2b.sshUser = "alice";
              networking.hostName = lib.mkDefault "corp";
              users.users.alice = { isNormalUser = true; uid = 1000; };
            };
          };
        };
      })
    ];
  };
  cfg = nixos.config;
  workload = builtins.head cfg.d2b._index.workloads.enabledList;
  workloadRow = builtins.head
    (import ../../../../nixos-modules/workload-process-rows.nix {
      config = cfg;
      inherit lib pkgs;
    });
  guest = cfg.d2b._computedWorkloads.${workload.workloadId}.config;
  credentialShare = lib.findFirst
    (share: share.tag == "d2b-gctl")
    null
    workloadRow.shares;
  credentialRow =
    builtins.head cfg.d2b._workloadGuestSessionCredentialRows;
  service = guest.systemd.services.d2b-guestd;
  storageRows = cfg.d2b._bundle.storageJson.data.paths;
  credentialStorage = lib.findFirst
    (row: row.id == credentialRow.resourceRef)
    null
    storageRows;
  credentialDirectoryStorage = lib.findFirst
    (row:
      row.id
        == "path:workload-guest-session:${credentialRow.workloadId}")
    null
    storageRows;
  credentialProfile = lib.findFirst
    (profile: profile.data.principal == credentialRow.readerPrincipal)
    null
    (lib.attrValues cfg.d2b._bundle.minijailProfiles);
  privilegesJson = builtins.toJSON cfg.d2b._bundle.privilegesJson.data;
  realmControllersJson =
    builtins.toJSON cfg.d2b._bundle.realmControllersJson.data;
  bundleJson = builtins.toJSON cfg.d2b._bundle.bundle.data;
  directGuestControlOptions =
    (import ../../../../nixos-modules/guest-control.nix {
      config = { };
      d2bInputs = null;
      inherit lib pkgs;
      name = "direct-consumer";
    }).options.d2b.guestControl;
  execStart = service.serviceConfig.ExecStart;
  pathAt = path:
    lib.findFirst (row: row.pathTemplate == path) null storageRows;
  grantFor = row:
    lib.findFirst
      (grant:
        grant.principal.kind == "group"
        && grant.principal.value == credentialRow.readerPrincipal)
      null
      row.accessAcl;
  realmRuntimeRoot = pathAt "/run/d2b/r";
  realmRuntime = pathAt "/run/d2b/r/${credentialRow.realmId}";
  realmWorkloads = pathAt "/run/d2b/r/${credentialRow.realmId}/w";
  workloadRuntime = pathAt workloadRow.runtimeRoot;
  runtimeRootRule = "d /run/d2b 1770 root d2b -";
  runtimeRootGrant =
    "a+ /run/d2b - - - - g:${credentialRow.readerPrincipal}:--x";
in
assert credentialShare != null;
assert credentialShare.source == "${workloadRow.runtimeRoot}/guest-session";
assert credentialShare.mountPoint == "/run/d2b-guest-control-host";
assert credentialShare.readOnly;
assert credentialRow.workloadId == workload.workloadId;
assert credentialRow.roleId
  == (builtins.head
    (builtins.filter
      (role: role.roleKind == "virtiofsd")
      workloadRow.roles)).roleId;
assert credentialRow.target
  == "${workloadRow.runtimeRoot}/guest-session/d2b-guest-session-v2";
assert credentialRow.creator == workloadRow.broker;
assert credentialRow.repairOwner == workloadRow.broker;
assert credentialRow.materializedByHostActivation == false;
assert credentialStorage != null;
assert credentialDirectoryStorage != null;
assert credentialStorage.owner.value == "root";
assert credentialStorage.group.kind == "group";
assert credentialStorage.group.value == credentialRow.readerPrincipal;
assert cfg.users.groups.${credentialRow.readerPrincipal}.gid
  == credentialRow.readerGid;
assert credentialStorage.mode == "0440";
assert credentialStorage.lifecycle == "process-scoped";
assert credentialStorage.persistence == "process-scoped";
assert credentialStorage.restartPolicy == "recreate-after-owner-death";
assert credentialStorage.adoptionPolicy == "quarantine-on-ambiguity";
assert credentialStorage.repairPolicy == "broker-fail-closed";
assert credentialStorage.sensitivity == "secret-adjacent";
assert credentialDirectoryStorage.owner.value == "root";
assert credentialDirectoryStorage.mode == "0750";
assert builtins.elem runtimeRootRule cfg.systemd.tmpfiles.rules;
assert builtins.elem runtimeRootGrant cfg.systemd.tmpfiles.rules;
assert realmRuntimeRoot != null;
assert realmRuntime != null;
assert realmWorkloads != null;
assert workloadRuntime != null;
assert (grantFor realmRuntimeRoot).permissions == "x";
assert (grantFor realmRuntime).permissions == "x";
assert (grantFor realmWorkloads).permissions == "x";
assert (grantFor workloadRuntime).permissions == "x";
assert credentialProfile != null;
assert builtins.elem credentialShare.source
  credentialProfile.data.mountPolicy.readOnlyPaths;
assert !(builtins.elem credentialShare.source
  credentialProfile.data.mountPolicy.writablePaths);
assert service.serviceConfig.LoadCredential
  == [
    "d2b-guest-session-v2:/run/d2b-guest-control-host/d2b-guest-session-v2"
  ];
assert lib.hasInfix
  "--workload-id ${workload.workloadId}"
  execStart;
assert !(lib.hasInfix "guest_control_token" execStart);
assert builtins.elem "/run/d2b-guest-control-host"
  service.unitConfig.RequiresMountsFor;
assert !(lib.hasInfix "GuestControlSign" privilegesJson);
assert !(lib.hasInfix "GuestSessionCredentialV1" realmControllersJson);
assert !(lib.hasInfix "d2b-guest-session-v2" realmControllersJson);
assert !(lib.hasInfix "d2b-guest-session-v2" bundleJson);
{
  inherit
    (credentialRow)
    authority
    bundleArtifact
    creator
    derivationMaterial
    format
    guestDelivery
    hostMaterialization
    lifecycle
    materializedByHostActivation
    observability
    payloadContract
    repairOwner
    resourceRef
    runtimeDependency
    schemaVersion
    ;
  inherit (credentialShare) mountPoint readOnly;
  hostStorage = {
    directoryMode = credentialDirectoryStorage.mode;
    groupIsWorkloadPrincipal =
      credentialStorage.group.kind == "group"
        && credentialStorage.group.value
          == credentialRow.readerPrincipal;
    mode = credentialStorage.mode;
    owner = credentialStorage.owner.value;
  };
  minijailReadOnly =
    builtins.elem credentialShare.source
      credentialProfile.data.mountPolicy.readOnlyPaths;
  sourceIsCanonicalRuntime =
    credentialShare.source == "${workloadRow.runtimeRoot}/guest-session";
  noCredentialInArtifacts =
    !(lib.hasInfix "d2b-guest-session-v2"
      (realmControllersJson + bundleJson));
  noGuestControlSign = !(lib.hasInfix "GuestControlSign" privilegesJson);
  authorityIsChildRealm =
    credentialRow.authority.generation
      == "d2bd-r-${credentialRow.realmId}"
    && credentialRow.authority.materialization
      == "d2bbr-r-${credentialRow.realmId}"
    && credentialRow.creator == credentialRow.authority.materialization
    && credentialRow.repairOwner
      == credentialRow.authority.materialization;
  directDefaults = {
    credentialName =
      directGuestControlOptions.sessionCredential.name.default;
    credentialSourcePath =
      directGuestControlOptions.sessionCredential.sourcePath.default;
    workloadId = directGuestControlOptions.workloadId.default;
  };
  pairedRuntimeWiring = {
    hasCredential =
      service.serviceConfig.LoadCredential
        == [
          "d2b-guest-session-v2:/run/d2b-guest-control-host/d2b-guest-session-v2"
        ];
    hasCanonicalWorkloadId =
      lib.hasInfix "--workload-id ${workload.workloadId}" execStart;
    legacyTokenAbsent = !(lib.hasInfix "guest_control_token"
      (builtins.toJSON service.serviceConfig.LoadCredential + execStart));
    dependencyBlocked =
      credentialRow.runtimeDependency.status == "blocked"
      && credentialRow.runtimeDependency.requiredTogether
        == [ "load-credential" "workload-id" ]
      && !credentialRow.runtimeDependency.standalone;
  };
  readerPrincipal = credentialRow.readerPrincipal;
  traversal = [
    {
      path = "/run/d2b";
      owner = "root";
      group = "d2b";
      mode = "1770";
      grant = if builtins.elem runtimeRootGrant cfg.systemd.tmpfiles.rules
        then "x"
        else null;
    }
    {
      path = realmRuntimeRoot.pathTemplate;
      owner = realmRuntimeRoot.owner.value;
      group = realmRuntimeRoot.group.value;
      mode = realmRuntimeRoot.mode;
      grant = (grantFor realmRuntimeRoot).permissions;
    }
    {
      path = realmRuntime.pathTemplate;
      owner = realmRuntime.owner.value;
      group = realmRuntime.group.value;
      mode = realmRuntime.mode;
      grant = (grantFor realmRuntime).permissions;
    }
    {
      path = realmWorkloads.pathTemplate;
      owner = realmWorkloads.owner.value;
      group = realmWorkloads.group.value;
      mode = realmWorkloads.mode;
      grant = (grantFor realmWorkloads).permissions;
    }
    {
      path = workloadRuntime.pathTemplate;
      owner = workloadRuntime.owner.value;
      group = workloadRuntime.group.value;
      mode = workloadRuntime.mode;
      grant = (grantFor workloadRuntime).permissions;
    }
    {
      path = credentialDirectoryStorage.pathTemplate;
      owner = credentialDirectoryStorage.owner.value;
      group = credentialDirectoryStorage.group.value;
      mode = credentialDirectoryStorage.mode;
      grant = "group-rx";
    }
    {
      path = credentialStorage.pathTemplate;
      owner = credentialStorage.owner.value;
      group = credentialStorage.group.value;
      mode = credentialStorage.mode;
      grant = "group-read";
    }
  ];
  workloadId = workload.workloadId;
}
