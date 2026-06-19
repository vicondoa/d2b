{ config, lib, pkgs, ... }:

let
  cfg = config.nixling;
  processDags = cfg._bundle.processesJson.data.vms or [ ];

  privateEtc = source: {
    inherit source;
    mode = "0640";
    user = "root";
    group = if cfg.daemonExperimental.enable then "nixlingd" else "root";
  };

  actor = kind: value: { inherit kind value; };
  principal = kind: value: { inherit kind value; };
  uidPrincipal = uid: principal "uid" (toString uid);
  gidPrincipal = gid: principal "gid" (toString gid);

  pathId = prefix: path: "${prefix}:${builtins.hashString "sha256" (builtins.unsafeDiscardStringContext path)}";
  modeForKind = kind:
    if kind == "unix-socket" then "0660"
    else if kind == "regular-file" then "0640"
    else "0750";
  modeString = mode:
    if mode == 384 then "0600"
    else if mode == 416 then "0640"
    else if mode == 432 then "0660"
    else toString mode;

  mkPath =
    {
      id,
      scope,
      path,
      kind ? "directory",
      lifecycle ? "persistent",
      persistence ? "persistent",
      owner ? principal "user" "nixlingd",
      group ? principal "group" "nixlingd",
      mode ? modeForKind kind,
      creator ? actor "broker" "nixling-priv-broker",
      writers ? [ creator ],
      readers ? [ (actor "daemon" "nixlingd") ],
      cleanupPolicy ? "never",
      repairPolicy ? "broker-reconcile",
      restartPolicy ? "preserve-across-daemon-restart",
      adoptionPolicy ? "adopt-with-live-owner-proof",
      leaseClass ? "none",
      sensitivity ? "private",
      noFollow ? true,
      recursive ? false,
      invariants ? [ "no-symlink" "broker-opaque-id-only" ],
    }:
    {
      inherit
        id
        scope
        kind
        lifecycle
        persistence
        owner
        group
        mode
        creator
        writers
        readers
        cleanupPolicy
        repairPolicy
        restartPolicy
        adoptionPolicy
        leaseClass
        sensitivity
        noFollow
        recursive
        invariants
        ;
      pathTemplate = path;
      accessAcl = [ ];
      defaultAcl = [ ];
    };

  bundleArtifactPaths = [
    "/etc/nixling/bundle.json"
    "/etc/nixling/host.json"
    "/etc/nixling/processes.json"
    "/etc/nixling/privileges.json"
    "/etc/nixling/storage.json"
    "/etc/nixling/sync.json"
  ];

  basePaths = [
    (mkPath {
      id = "path:etc-root";
      scope = "host";
      path = "/etc/nixling";
      lifecycle = "config";
      persistence = "persistent";
      owner = principal "user" "root";
      group = principal "group" "nixlingd";
      creator = actor "nix-module" "environment.etc";
      writers = [ (actor "nix-module" "environment.etc") ];
      cleanupPolicy = "never";
      repairPolicy = "nix-activation";
      restartPolicy = "not-applicable";
      adoptionPolicy = "not-adoptable";
    })
    (mkPath {
      id = "path:state-root";
      scope = "host";
      path = toString cfg.site.stateDir;
      persistence = "persistent";
      owner = principal "user" "root";
      group = principal "group" "nixlingd";
      creator = actor "nix-module" "tmpfiles";
    })
    (mkPath {
      id = "path:run-root";
      scope = "host";
      path = "/run/nixling";
      lifecycle = "boot-scoped-readoptable";
      persistence = "boot-scoped";
      owner = principal "user" "nixlingd";
      group = principal "group" "nixling";
      mode = "0750";
      creator = actor "nix-module" "tmpfiles";
      cleanupPolicy = "boot";
      leaseClass = "process-pidfd";
    })
  ] ++ map
    (path: mkPath {
      id = pathId "path:artifact" path;
      scope = "host";
      inherit path;
      kind = "regular-file";
      lifecycle = "config";
      persistence = "persistent";
      owner = principal "user" "root";
      group = principal "group" "nixlingd";
      creator = actor "nix-module" "environment.etc";
      writers = [ (actor "nix-module" "environment.etc") ];
      cleanupPolicy = "never";
      repairPolicy = "nix-activation";
      restartPolicy = "not-applicable";
      adoptionPolicy = "not-adoptable";
      sensitivity = "private";
    })
    bundleArtifactPaths;

  nodeWritablePaths = lib.flatten (map
    (dag: lib.flatten (map
      (node: map
        (writable:
          mkPath {
            id = pathId "path:writable" "${dag.vm}:${node.id}:${writable.path}";
            scope = "role:${dag.vm}:${node.id}";
            path = writable.path;
            lifecycle =
              if lib.hasPrefix "/run/" writable.path then "boot-scoped-readoptable" else "persistent";
            persistence =
              if lib.hasPrefix "/run/" writable.path then "boot-scoped" else "persistent";
            owner = uidPrincipal node.profile.uid;
            group = gidPrincipal node.profile.gid;
            creator = actor "broker" "nixling-priv-broker";
            writers = [ (actor "role" "role:${dag.vm}:${node.id}") ];
            readers = [
              (actor "daemon" "nixlingd")
              (actor "role" "role:${dag.vm}:${node.id}")
            ];
            cleanupPolicy =
              if lib.hasPrefix "/run/" writable.path then "process-exit-with-proof" else "never";
            repairPolicy = "broker-reconcile";
            leaseClass =
              if lib.hasPrefix "/run/" writable.path then "process-pidfd" else "none";
            invariants = [ "no-symlink" "broker-opaque-id-only" ];
          })
        (node.profile.mountPolicy.writablePaths or [ ]))
      (dag.nodes or [ ])))
    processDags);

  readinessSocketPaths = lib.flatten (map
    (dag: lib.flatten (map
      (node: map
        (ready:
          mkPath {
            id = pathId "path:readiness" "${dag.vm}:${node.id}:${ready.value}";
            scope = "role:${dag.vm}:${node.id}";
            path = ready.value;
            kind = "unix-socket";
            lifecycle = "boot-scoped-readoptable";
            persistence = "boot-scoped";
            owner = uidPrincipal node.profile.uid;
            group = gidPrincipal node.profile.gid;
            creator = actor "role" "role:${dag.vm}:${node.id}";
            writers = [ (actor "role" "role:${dag.vm}:${node.id}") ];
            readers = [
              (actor "daemon" "nixlingd")
              (actor "broker" "nixling-priv-broker")
            ];
            cleanupPolicy = "process-exit-with-proof";
            leaseClass = "process-pidfd";
            sensitivity = "private";
          })
        (lib.filter
          (ready: builtins.elem (ready.kind or "") [ "unix-socket-exists" "unix-socket-listening" ])
          (node.readiness or [ ])))
      (dag.nodes or [ ])))
    processDags);

  diskInitPaths = lib.flatten (map
    (dag: lib.flatten (map
      (node: map
        (op:
          mkPath {
            id = pathId "path:disk-init" "${dag.vm}:${node.id}:${op.targetPath}";
            scope = "role:${dag.vm}:${node.id}";
            path = op.targetPath;
            kind = "regular-file";
            lifecycle = "persistent";
            persistence = "persistent";
            owner = uidPrincipal op.ownerUid;
            group = gidPrincipal op.ownerGid;
            mode = modeString op.mode;
            creator = actor "broker" "nixling-priv-broker";
            writers = [ (actor "broker" "nixling-priv-broker") ];
            readers = [ (actor "role" "role:${dag.vm}:cloud-hypervisor") ];
            cleanupPolicy = "never";
            repairPolicy = "broker-reconcile";
            leaseClass = "none";
            invariants = [ "no-symlink" "broker-opaque-id-only" "root-owned-parent" ];
          })
        (lib.filter (op: (op.kind or "") == "diskInit") (node.planOps or [ ])))
      (dag.nodes or [ ])))
    processDags);

  adoptableRoles = [
    "audio"
    "cloud-hypervisor-runner"
    "gpu"
    "gpu-render-node"
    "otel-host-bridge"
    "swtpm"
    "usbip"
    "video"
    "virtiofsd"
    "vsock-relay"
    "wayland-proxy"
  ];

  restartPolicyFor = dag: node:
    let
      role = node.role;
      adoptable = builtins.elem role adoptableRoles;
    in {
      vm = dag.vm;
      roleId = node.id;
      restartClass = if adoptable then "adoptable" else "recreatable";
      adoptionInputs = {
        cgroupLeaf = node.profile.cgroupPlacement.subtree or null;
        identityChecks = lib.optionals adoptable [
          "cgroup-membership"
          "executable-path"
          "profile-id"
          "pidfd-open-after-candidate-read"
        ];
      };
      persistentStateRefs = [ ];
      runtimeStateRefs = [ ];
      cleanupBeforeRestart = false;
      degradeOnFailure = if adoptable then "adoption-quarantined" else "restart-required";
      degradeScope = "role";
      readinessAfterAdopt = {
        kind = if adoptable then "existing-predicate" else "none";
        storageRef = null;
      };
      remediationId = if adoptable then "remediate:vm-status" else "remediate:vm-restart";
    };

  degradedStates = map
    (reason: {
      inherit reason;
      scope = "role";
      storageClass = "tamper-evident-segmented";
      remediationId = "remediate:host-doctor";
    })
    [
      "storage-drift"
      "storage-repair-failed"
      "adoption-pending"
      "adoption-quarantined"
      "restart-required"
      "lock-owner-ambiguous"
      "lock-acquire-timeout"
      "external-dependency-unhealthy"
      "migration-required"
      "migration-failed"
      "violation-audit-throttled"
    ];

  data = {
    schemaVersion = "v2";
    roots = [
      {
        id = "root:etc";
        path = "/etc/nixling";
        class = "config";
        owner = principal "user" "root";
        group = principal "group" "nixlingd";
        mode = "0750";
        authority = "nix-module";
      }
      {
        id = "root:state";
        path = toString cfg.site.stateDir;
        class = "persistent";
        owner = principal "user" "root";
        group = principal "group" "nixlingd";
        mode = "0750";
        authority = "broker";
      }
      {
        id = "root:run";
        path = "/run/nixling";
        class = "runtime";
        owner = principal "user" "nixlingd";
        group = principal "group" "nixling";
        mode = "0750";
        authority = "daemon";
      }
    ];
    paths = basePaths ++ nodeWritablePaths ++ readinessSocketPaths ++ diskInitPaths;
    restartPolicies = lib.flatten (map (dag: map (node: restartPolicyFor dag node) (dag.nodes or [ ])) processDags);
    degradedStates = degradedStates;
    remediations = [
      {
        id = "remediate:host-doctor";
        command = "nixling host doctor --storage --read-only";
        description = "Inspect storage/degraded state without mutating the host.";
      }
      {
        id = "remediate:vm-status";
        command = "nixling vm status <vm>";
        description = "Inspect the VM's role-level degraded state and adoption evidence.";
      }
      {
        id = "remediate:vm-restart";
        command = "nixling vm restart <vm> --apply";
        description = "Restart a VM whose role cannot be safely re-adopted.";
      }
    ];
  };

  jsonText = builtins.toJSON data;
  jsonFile = pkgs.writeText "nixling-storage.json" jsonText;
in
{
  options.nixling._bundle.storageJson = lib.mkOption {
    type = lib.types.unspecified;
    readOnly = true;
    internal = true;
    description = "Internal schema-v2 storage lifecycle artifact metadata.";
  };

  config = {
    nixling._bundle.storageJson = {
      inherit data jsonText;
      path = "${jsonFile}";
    };
    environment.etc."nixling/storage.json" = privateEtc jsonFile;
  };
}
