{ mkEval, lib, pkgs, ... }:

let
  digestHelpers =
    import ../../../../nixos-modules/resources-bundle.nix { inherit lib; };
  compilerCommand = "d2b-resource-compiler";
  compilerStub = pkgs.writeShellScriptBin
    compilerCommand
    "exit 0";
  catalogData = {
    schemaVersion = 3;
    entries = [ ];
  };
  catalogPreimageJson = builtins.toJSON catalogData;
  catalogDigest = "sha256:${digestHelpers.framedDigest
    "d2b:v3:artifact-catalog"
    catalogPreimageJson}";
  catalogDocument = catalogData // { catalogDigest = catalogDigest; };
  catalogJson = builtins.toJSON catalogDocument;
  catalogPath = pkgs.writeText "d2b-artifact-catalog-eval-fixture"
    "${catalogJson}\n";
  catalogOverride = { lib, ... }: {
    d2b._artifactCatalogV3 = lib.mkForce {
      ids = [ ];
      artifactRows = [ ];
      preimage = catalogData;
      preimageJson = catalogPreimageJson;
      inherit catalogDigest;
      catalogData = catalogDocument;
      catalogJson = catalogJson;
      path = catalogPath;
      publicEntries = [ ];
    };
    d2b._bundle.extraArtifacts.artifactCatalog = lib.mkOverride 0 {
      data = catalogData;
      jsonText = catalogJson;
      path = catalogPath;
      installFileName = "artifact-catalog.json";
      classification = "contractPrivateNonSecret";
      sensitivity = "nonSecret";
    };
  };
  mkEvalStub = modules: mkEval (modules ++ [
    ({ lib, ... }: {
      d2b._resourceCompiler.phase2.compiler = lib.mkForce compilerStub;
    })
    catalogOverride
  ]);
  catalogShape = import ../../../../nixos-modules/generated/provider-catalog-shape.nix;
  catalogEntry = name:
    let
      digestFields = lib.listToAttrs (map
        (field: lib.nameValuePair field
          "sha256:${builtins.hashString "sha256" "${name}/${field}"}")
        catalogShape.digestFields);
      plainFields = lib.listToAttrs (map
        (field: lib.nameValuePair field "${name}/${field}")
        (lib.filter (field: !(builtins.elem field catalogShape.digestFields))
          catalogShape.fields));
    in
    digestFields // plainFields;

  artifact = name: type: {
    package = pkgs.writeText name name;
    inherit type;
    catalog = catalogEntry name;
  };

  host = { ... }: {
    boot.loader.grub.enable = false;
    boot.loader.systemd-boot.enable = false;
    boot.initrd.includeDefaultModules = false;
    fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
    environment.etc."machine-id".text = "00000000000000000000000000000000";
    system.stateVersion = "25.11";
    users.users.alice = { isNormalUser = true; uid = 1000; };
    d2b.site = {
      waylandUser = "alice";
      launcherUsers = [ "alice" ];
    };
    d2b.artifacts = {
      provider-network-local = artifact "provider-network-local" "provider";
      net-vm-base = artifact "net-vm-base" "nixos-system";
    };
  };

  provider = {
    type = "Provider";
    spec.artifactId = "provider-network-local";
  };
  network = {
    type = "Network";
    spec = {
      providerRef = "Provider/network-local";
      lanCidr = "10.20.0.0/24";
      uplinkCidr = "192.0.2.0/30";
      netVmSystemArtifactId = "net-vm-base";
    };
  };
  generation = includeNetwork: (mkEvalStub [ host {
    d2b.zones.work.resources = {
      network-local = provider;
    } // lib.optionalAttrs includeNetwork {
      main = network;
    };
  } ]).config;

  first = generation true;
  second = generation false;
  redeclared = generation true;
  firstBundle = first.d2b._bundle.zoneResourceBundles.work;
  secondBundle = second.d2b._bundle.zoneResourceBundles.work;
  redeclaredBundle = redeclared.d2b._bundle.zoneResourceBundles.work;
  firstTypes = map (resource: resource.type)
    firstBundle.data.resources;
  secondTypes = map (resource: resource.type)
    secondBundle.data.resources;
  redeclaredTypes = map (resource: resource.type)
    redeclaredBundle.data.resources;
  cleanup = first.d2b._resourceCompiler.zones.work;
  compilerSelected =
    let
      selected = first.d2b._resourceCompiler.phase2.compiler;
      selectedPath =
        builtins.unsafeDiscardStringContext (toString selected);
      stubPath = builtins.unsafeDiscardStringContext (toString compilerStub);
    in
    selectedPath == stubPath;
in
{
  "generation-cleanup-absent-network/compiler-contract" = {
    expr = {
      fakeCompilerSelected = compilerSelected;
      networkRemoved = builtins.elem "Network" firstTypes
        && !(builtins.elem "Network" secondTypes);
      identicalNetworkRedeclared = builtins.elem "Network" firstTypes
        && builtins.elem "Network" redeclaredTypes
        && firstBundle.data.resources == redeclaredBundle.data.resources
        && firstBundle.data.contentHash == redeclaredBundle.data.contentHash;
      removedNetworkChangesArtifact =
        firstBundle.data.contentHash != secondBundle.data.contentHash;
      absentResourceAction = cleanup.transition.absentResourceAction;
      pendingCondition = cleanup.transition.pendingCondition;
      directDeleteOwner = cleanup.ownership.eligibleValue;
      preservedOwners = cleanup.ownership.preservedValues;
      retainedGenerations = cleanup.retainedGenerations;
      cleanupBlocksActivation = cleanup.transition.cleanupBlocksActivation;
    };
    expected = {
      fakeCompilerSelected = true;
      networkRemoved = true;
      identicalNetworkRedeclared = true;
      removedNetworkChangesArtifact = true;
      absentResourceAction = "delete";
      pendingCondition = "PendingCleanup";
      directDeleteOwner = "configuration";
      preservedOwners = [ "controller" "api" ];
      retainedGenerations = 3;
      cleanupBlocksActivation = false;
    };
  };
}
