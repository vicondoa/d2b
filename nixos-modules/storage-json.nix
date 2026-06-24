{ config, lib, ... }:

let
  cfg = config.nixling;
  nl = import ./lib.nix { inherit lib; };
  enabledVms = lib.filterAttrs (_: vm: vm.enable) cfg.vms;
  normalNixosVms = nl.normalNixosVms cfg.vms;
  qemuMediaVms = nl.qemuMediaVms cfg.vms;
  tpmVms = lib.filterAttrs (_: vm: vm.tpm.enable) normalNixosVms;
  processDags = cfg._bundle.processesJson.data.vms or [ ];

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

  daemonStateReports = [
    "pidfd-table.json"
    "kernel-module-report.json"
    "autostart-report.json"
    "storage-lifecycle-report.json"
    "shutdown-degraded.json"
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

  hostMutablePaths = [
    (mkPath {
      id = "path:daemon-state";
      scope = "host";
      path = "${toString cfg.site.stateDir}/daemon-state";
      owner = principal "user" "nixlingd";
      group = principal "group" "nixlingd";
      mode = "0755";
      creator = actor "nix-module" "tmpfiles";
      writers = [ (actor "daemon" "nixlingd") ];
      cleanupPolicy = "never";
      repairPolicy = "nix-activation";
    })
    (mkPath {
      id = "path:run-locks";
      scope = "host";
      path = "/run/nixling/locks";
      lifecycle = "boot-scoped-readoptable";
      persistence = "boot-scoped";
      owner = principal "user" "nixlingd";
      group = principal "group" "nixlingd";
      mode = "0700";
      creator = actor "nix-module" "tmpfiles";
      writers = [ (actor "daemon" "nixlingd") ];
      cleanupPolicy = "boot";
      repairPolicy = "nix-activation";
      leaseClass = "none";
    })
    (mkPath {
      id = "path:run-locks-usbip";
      scope = "host";
      path = "/run/nixling/locks/usbip";
      lifecycle = "boot-scoped-readoptable";
      persistence = "boot-scoped";
      owner = principal "user" "root";
      group = principal "group" "nixlingd";
      mode = "0750";
      creator = actor "nix-module" "tmpfiles";
      writers = [ (actor "broker" "nixling-priv-broker") ];
      cleanupPolicy = "boot";
      repairPolicy = "nix-activation";
      leaseClass = "file-record";
      invariants = [ "no-symlink" "broker-opaque-id-only" "scope-authorization-required" ];
    })
    (mkPath {
      id = "path:run-state";
      scope = "host";
      path = "/run/nixling/state";
      lifecycle = "boot-scoped-readoptable";
      persistence = "boot-scoped";
      owner = principal "user" "nixlingd";
      group = principal "group" "nixlingd";
      mode = "0700";
      creator = actor "nix-module" "tmpfiles";
      writers = [ (actor "daemon" "nixlingd") ];
      cleanupPolicy = "boot";
      repairPolicy = "nix-activation";
    })
    (mkPath {
      id = "path:run-otel";
      scope = "host";
      path = "/run/nixling/otel";
      lifecycle = "boot-scoped-readoptable";
      persistence = "boot-scoped";
      owner = principal "user" "nixlingd";
      group = principal "group" "nixling";
      mode = "0750";
      creator = actor "nix-module" "tmpfiles";
      writers = [
        (actor "daemon" "nixlingd")
        (actor "role" "role:host:otel-host-bridge")
      ];
      readers = [
        (actor "daemon" "nixlingd")
        (actor "role" "role:host:otel-host-bridge")
      ];
      cleanupPolicy = "boot";
      repairPolicy = "nix-activation";
      leaseClass = "process-pidfd";
      invariants = [ "no-symlink" "scope-authorization-required" ];
    })
    (mkPath {
      id = "path:state-ledgers";
      scope = "host";
      path = "${toString cfg.site.stateDir}/state";
      owner = principal "user" "root";
      group = principal "group" "nixlingd";
      mode = "0750";
      creator = actor "broker" "nixling-priv-broker";
      writers = [ (actor "broker" "nixling-priv-broker") ];
      readers = [
        (actor "daemon" "nixlingd")
        (actor "broker" "nixling-priv-broker")
      ];
      cleanupPolicy = "never";
      repairPolicy = "broker-reconcile";
    })
  ] ++ lib.optionals (qemuMediaVms != { }) [
    (mkPath {
      id = "path:qemu-media-registry";
      scope = "host";
      path = "${toString cfg.site.stateDir}/media-registry";
      owner = principal "user" "root";
      group = principal "group" "root";
      mode = "0700";
      creator = actor "broker" "nixling-priv-broker";
      writers = [ (actor "broker" "nixling-priv-broker") ];
      readers = [ (actor "broker" "nixling-priv-broker") ];
      cleanupPolicy = "never";
      repairPolicy = "broker-fail-closed";
      invariants = [ "no-symlink" "root-owned-parent" "broker-opaque-id-only" "scope-authorization-required" ];
    })
    (mkPath {
      id = "path:qemu-media-redacted-index";
      scope = "host";
      path = "/run/nixling/qemu-media-registry-index.json";
      kind = "regular-file";
      lifecycle = "boot-scoped-readoptable";
      persistence = "boot-scoped";
      owner = principal "user" "root";
      group = principal "group" "root";
      mode = "0644";
      creator = actor "broker" "nixling-priv-broker";
      writers = [ (actor "broker" "nixling-priv-broker") ];
      readers = [
        (actor "daemon" "nixlingd")
        (actor "broker" "nixling-priv-broker")
        (actor "operator" "host-doctor")
      ];
      cleanupPolicy = "boot";
      repairPolicy = "broker-reconcile";
      sensitivity = "private";
      invariants = [ "no-symlink" "broker-opaque-id-only" ];
    })
  ] ++ lib.optionals (tpmVms != { }) [
    (mkPath {
      id = "path:swtpm-marker-root";
      scope = "host";
      path = "${toString cfg.site.stateDir}/swtpm-markers";
      owner = principal "user" "root";
      group = principal "group" "root";
      mode = "0700";
      creator = actor "broker" "nixling-priv-broker";
      writers = [ (actor "broker" "nixling-priv-broker") ];
      readers = [ (actor "broker" "nixling-priv-broker") ];
      cleanupPolicy = "never";
      repairPolicy = "broker-fail-closed";
      sensitivity = "secret-adjacent";
      invariants = [ "no-symlink" "broker-opaque-id-only" "root-owned-parent" ];
    })
  ] ++ map
    (file: mkPath {
      id = "path:daemon-state:${file}";
      scope = "host";
      path = "${toString cfg.site.stateDir}/daemon-state/${file}";
      kind = "regular-file";
      owner = principal "user" "nixlingd";
      group = principal "group" "nixlingd";
      mode = "0644";
      creator = actor "daemon" "nixlingd";
      writers = [ (actor "daemon" "nixlingd") ];
      readers = [
        (actor "daemon" "nixlingd")
        (actor "operator" "host-doctor")
      ];
      cleanupPolicy = "never";
      repairPolicy = "none";
      invariants = [ "no-symlink" "no-recursive-mutation" ];
    })
    daemonStateReports;

  perNormalVmStoragePaths = lib.flatten (lib.mapAttrsToList
    (name: _: [
      (mkPath {
        id = "path:vm-state:${name}";
        scope = "vm:${name}";
        path = "${toString cfg.store.stateDir}/${name}";
        owner = principal "user" "nixlingd";
        group = principal "group" "users";
        mode = "3770";
        creator = actor "nix-module" "tmpfiles";
        writers = [
          (actor "daemon" "nixlingd")
          (actor "broker" "nixling-priv-broker")
        ];
        readers = [
          (actor "daemon" "nixlingd")
          (actor "broker" "nixling-priv-broker")
        ];
        cleanupPolicy = "never";
        repairPolicy = "nix-activation";
        invariants = [ "no-symlink" "root-owned-parent" "scope-authorization-required" ];
      })
      (mkPath {
        id = "path:vm-run:${name}";
        scope = "vm:${name}";
        path = "/run/nixling/vms/${name}";
        lifecycle = "boot-scoped-readoptable";
        persistence = "boot-scoped";
        owner = principal "user" "nixlingd";
        group = principal "group" "nixling";
        mode = "0750";
        creator = actor "nix-module" "tmpfiles";
        writers = [
          (actor "daemon" "nixlingd")
          (actor "broker" "nixling-priv-broker")
        ];
        cleanupPolicy = "boot";
        repairPolicy = "nix-activation";
        leaseClass = "process-pidfd";
      })
      (mkPath {
        id = "path:vm-run-guest-control:${name}";
        scope = "vm:${name}";
        path = "/run/nixling/vms/${name}/guest-control";
        lifecycle = "boot-scoped-readoptable";
        persistence = "boot-scoped";
        owner = principal "user" "nixlingd";
        group = principal "group" "nixling";
        mode = "0750";
        creator = actor "nix-module" "tmpfiles";
        writers = [
          (actor "daemon" "nixlingd")
          (actor "broker" "nixling-priv-broker")
        ];
        cleanupPolicy = "boot";
        repairPolicy = "nix-activation";
        leaseClass = "process-pidfd";
      })
      (mkPath {
        id = "path:daemon-state-vm:${name}";
        scope = "vm:${name}";
        path = "${toString cfg.site.stateDir}/daemon-state/${name}";
        owner = principal "user" "nixlingd";
        group = principal "group" "nixlingd";
        mode = "0755";
        creator = actor "daemon" "nixlingd";
        writers = [ (actor "daemon" "nixlingd") ];
        readers = [ (actor "daemon" "nixlingd") ];
        cleanupPolicy = "never";
        repairPolicy = "none";
        invariants = [ "no-symlink" "no-recursive-mutation" ];
      })
      (mkPath {
        id = "path:daemon-state-vm-runtime:${name}";
        scope = "vm:${name}";
        path = "${toString cfg.site.stateDir}/daemon-state/${name}/runtime.<role>.json";
        kind = "regular-file";
        owner = principal "user" "nixlingd";
        group = principal "group" "nixlingd";
        mode = "0644";
        creator = actor "daemon" "nixlingd";
        writers = [ (actor "daemon" "nixlingd") ];
        readers = [ (actor "daemon" "nixlingd") ];
        cleanupPolicy = "never";
        repairPolicy = "none";
        invariants = [ "no-symlink" "no-recursive-mutation" ];
      })
      (mkPath {
        id = "path:daemon-state-vm-api-ready:${name}";
        scope = "vm:${name}";
        path = "${toString cfg.site.stateDir}/daemon-state/${name}/api-ready.json";
        kind = "regular-file";
        owner = principal "user" "nixlingd";
        group = principal "group" "nixlingd";
        mode = "0644";
        creator = actor "daemon" "nixlingd";
        writers = [ (actor "daemon" "nixlingd") ];
        readers = [
          (actor "daemon" "nixlingd")
          (actor "operator" "vm-status")
        ];
        cleanupPolicy = "never";
        repairPolicy = "none";
        invariants = [ "no-symlink" "no-recursive-mutation" ];
      })
      (mkPath {
        id = "path:store-view:${name}";
        scope = "vm:${name}";
        path = "${toString cfg.store.stateDir}/${name}/store-view";
        owner = principal "user" "nixlingd";
        group = principal "group" "users";
        mode = "0755";
        creator = actor "nix-module" "tmpfiles";
        writers = [ (actor "broker" "nixling-priv-broker") ];
        readers = [
          (actor "daemon" "nixlingd")
          (actor "broker" "nixling-priv-broker")
          (actor "role" "role:${name}:virtiofsd")
        ];
        cleanupPolicy = "never";
        repairPolicy = "broker-reconcile";
        invariants = [ "no-symlink" "same-filesystem" "hardlink-farm-no-recursion" "broker-opaque-id-only" ];
      })
      (mkPath {
        id = "path:store-view-live:${name}";
        scope = "vm:${name}";
        path = "${toString cfg.store.stateDir}/${name}/store-view/live";
        owner = principal "user" "nixlingd";
        group = principal "group" "users";
        mode = "0755";
        creator = actor "nix-module" "tmpfiles";
        writers = [ (actor "broker" "nixling-priv-broker") ];
        readers = [
          (actor "daemon" "nixlingd")
          (actor "broker" "nixling-priv-broker")
          (actor "role" "role:${name}:virtiofsd")
        ];
        cleanupPolicy = "cutover-only";
        repairPolicy = "broker-reconcile";
        invariants = [ "no-symlink" "same-filesystem" "hardlink-farm-no-recursion" "broker-opaque-id-only" ];
      })
      (mkPath {
        id = "path:store-view-marker:${name}";
        scope = "vm:${name}";
        path = "${toString cfg.store.stateDir}/${name}/store-view/live/.nixling-marker-${name}";
        kind = "regular-file";
        owner = principal "user" "nixlingd";
        group = principal "group" "users";
        mode = "0444";
        creator = actor "broker" "nixling-priv-broker";
        writers = [ (actor "broker" "nixling-priv-broker") ];
        readers = [
          (actor "daemon" "nixlingd")
          (actor "role" "role:${name}:virtiofsd")
        ];
        cleanupPolicy = "never";
        repairPolicy = "broker-reconcile";
        invariants = [ "no-symlink" "same-filesystem" "hardlink-farm-no-recursion" "broker-opaque-id-only" ];
      })
      (mkPath {
        id = "path:store-view-meta:${name}";
        scope = "vm:${name}";
        path = "${toString cfg.store.stateDir}/${name}/store-view/meta";
        owner = principal "user" "nixlingd";
        group = principal "group" "users";
        mode = "0755";
        creator = actor "nix-module" "tmpfiles";
        writers = [ (actor "broker" "nixling-priv-broker") ];
        readers = [
          (actor "daemon" "nixlingd")
          (actor "broker" "nixling-priv-broker")
          (actor "role" "role:${name}:virtiofsd")
        ];
        cleanupPolicy = "never";
        repairPolicy = "broker-reconcile";
        invariants = [ "no-symlink" "same-filesystem" "hardlink-farm-no-recursion" "broker-opaque-id-only" ];
      })
      (mkPath {
        id = "path:store-view-generations:${name}";
        scope = "vm:${name}";
        path = "${toString cfg.store.stateDir}/${name}/store-view/meta/generations";
        owner = principal "user" "nixlingd";
        group = principal "group" "users";
        mode = "0755";
        creator = actor "broker" "nixling-priv-broker";
        writers = [ (actor "broker" "nixling-priv-broker") ];
        readers = [
          (actor "daemon" "nixlingd")
          (actor "role" "role:${name}:virtiofsd")
        ];
        cleanupPolicy = "cutover-only";
        repairPolicy = "broker-reconcile";
        invariants = [ "no-symlink" "same-filesystem" "hardlink-farm-no-recursion" "broker-opaque-id-only" ];
      })
      (mkPath {
        id = "path:store-view-gcroots:${name}";
        scope = "vm:${name}";
        path = "${toString cfg.store.stateDir}/${name}/store-view/meta/gcroots";
        owner = principal "user" "nixlingd";
        group = principal "group" "users";
        mode = "0755";
        creator = actor "broker" "nixling-priv-broker";
        writers = [ (actor "broker" "nixling-priv-broker") ];
        readers = [ (actor "daemon" "nixlingd") ];
        cleanupPolicy = "cutover-only";
        repairPolicy = "broker-reconcile";
        invariants = [ "no-symlink" "same-filesystem" "hardlink-farm-no-recursion" "broker-opaque-id-only" ];
      })
      (mkPath {
        id = "path:store-view-current:${name}";
        scope = "vm:${name}";
        path = "${toString cfg.store.stateDir}/${name}/store-view/meta/current";
        kind = "symlink";
        owner = principal "user" "nixlingd";
        group = principal "group" "users";
        mode = "0777";
        creator = actor "broker" "nixling-priv-broker";
        writers = [ (actor "broker" "nixling-priv-broker") ];
        readers = [
          (actor "daemon" "nixlingd")
          (actor "role" "role:${name}:virtiofsd")
        ];
        cleanupPolicy = "cutover-only";
        repairPolicy = "broker-reconcile";
        noFollow = false;
        invariants = [ "same-filesystem" "hardlink-farm-no-recursion" "broker-opaque-id-only" ];
      })
      (mkPath {
        id = "path:store-sync-lock:${name}";
        scope = "vm:${name}";
        path = "${toString cfg.store.stateDir}/${name}/store-view/sync.lock";
        kind = "regular-file";
        owner = principal "user" "nixlingd";
        group = principal "group" "users";
        mode = "0640";
        creator = actor "broker" "nixling-priv-broker";
        writers = [ (actor "broker" "nixling-priv-broker") ];
        readers = [
          (actor "daemon" "nixlingd")
          (actor "broker" "nixling-priv-broker")
        ];
        cleanupPolicy = "never";
        repairPolicy = "broker-reconcile";
        leaseClass = "none";
        invariants = [ "no-symlink" "same-filesystem" "broker-opaque-id-only" ];
      })
    ])
    normalNixosVms);

  perTpmStoragePaths = lib.flatten (lib.mapAttrsToList
    (name: _: [
      (mkPath {
        id = "path:swtpm-state:${name}";
        scope = "vm:${name}";
        path = "${toString cfg.store.stateDir}/${name}/swtpm";
        owner = uidPrincipal (nl.stablePrincipalId ("nixling-" + name + "-swtpm"));
        group = gidPrincipal (nl.stablePrincipalId ("nixling-" + name + "-swtpm"));
        mode = "0700";
        creator = actor "broker" "nixling-priv-broker";
        writers = [
          (actor "broker" "nixling-priv-broker")
          (actor "role" "role:${name}:swtpm")
        ];
        readers = [ (actor "broker" "nixling-priv-broker") ];
        cleanupPolicy = "never";
        repairPolicy = "broker-fail-closed";
        sensitivity = "secret-adjacent";
        invariants = [ "no-symlink" "broker-opaque-id-only" "scope-authorization-required" ];
      })
      (mkPath {
        id = "path:swtpm-marker:${name}";
        scope = "vm:${name}";
        path = "${toString cfg.site.stateDir}/swtpm-markers/${name}";
        kind = "regular-file";
        owner = principal "user" "root";
        group = principal "group" "root";
        mode = "0600";
        creator = actor "broker" "nixling-priv-broker";
        writers = [ (actor "broker" "nixling-priv-broker") ];
        readers = [ (actor "broker" "nixling-priv-broker") ];
        cleanupPolicy = "never";
        repairPolicy = "broker-fail-closed";
        sensitivity = "secret-adjacent";
        invariants = [ "no-symlink" "root-owned-parent" "broker-opaque-id-only" "scope-authorization-required" ];
      })
    ])
    tpmVms);

  perQemuMediaStoragePaths = lib.flatten (lib.mapAttrsToList
    (name: _: [
      (mkPath {
        id = "path:qemu-media-vm-state:${name}";
        scope = "vm:${name}";
        path = "${toString cfg.store.stateDir}/${name}";
        owner = principal "user" "nixling-${name}-qemu-media";
        group = principal "group" "nixling-${name}-qemu-media";
        mode = "0750";
        creator = actor "nix-module" "tmpfiles";
        writers = [
          (actor "broker" "nixling-priv-broker")
          (actor "role" "role:${name}:qemu-media")
        ];
        readers = [
          (actor "broker" "nixling-priv-broker")
          (actor "role" "role:${name}:qemu-media")
        ];
        cleanupPolicy = "never";
        repairPolicy = "nix-activation";
        invariants = [ "no-symlink" "root-owned-parent" "scope-authorization-required" ];
      })
      (mkPath {
        id = "path:qemu-media-vm-run:${name}";
        scope = "vm:${name}";
        path = "/run/nixling/vms/${name}";
        lifecycle = "boot-scoped-readoptable";
        persistence = "boot-scoped";
        owner = principal "user" "nixlingd";
        group = principal "group" "nixling";
        mode = "0750";
        creator = actor "nix-module" "tmpfiles";
        writers = [
          (actor "broker" "nixling-priv-broker")
          (actor "role" "role:${name}:qemu-media")
        ];
        cleanupPolicy = "boot";
        repairPolicy = "nix-activation";
        leaseClass = "process-pidfd";
        invariants = [ "no-symlink" "scope-authorization-required" ];
      })
      (mkPath {
        id = "path:qemu-media-qmp:${name}";
        scope = "vm:${name}";
        path = "/run/nixling/vms/${name}/qmp.sock";
        kind = "unix-socket";
        lifecycle = "boot-scoped-readoptable";
        persistence = "boot-scoped";
        owner = principal "user" "nixling-${name}-qemu-media";
        group = principal "group" "nixling-${name}-qemu-media";
        mode = "0660";
        creator = actor "role" "role:${name}:qemu-media";
        writers = [ (actor "role" "role:${name}:qemu-media") ];
        readers = [
          (actor "broker" "nixling-priv-broker")
          (actor "daemon" "nixlingd")
        ];
        cleanupPolicy = "process-exit-with-proof";
        repairPolicy = "broker-fail-closed";
        leaseClass = "process-pidfd";
        invariants = [ "no-symlink" "scope-authorization-required" ];
      })
      (mkPath {
        id = "path:qemu-media-registry-vm:${name}";
        scope = "vm:${name}";
        path = "${toString cfg.site.stateDir}/media-registry/${name}";
        owner = principal "user" "root";
        group = principal "group" "root";
        mode = "0700";
        creator = actor "broker" "nixling-priv-broker";
        writers = [ (actor "broker" "nixling-priv-broker") ];
        readers = [ (actor "broker" "nixling-priv-broker") ];
        cleanupPolicy = "never";
        repairPolicy = "broker-fail-closed";
        sensitivity = "secret-adjacent";
        invariants = [ "no-symlink" "root-owned-parent" "broker-opaque-id-only" "scope-authorization-required" ];
      })
      (mkPath {
        id = "path:qemu-media-registry-records:${name}";
        scope = "vm:${name}";
        path = "${toString cfg.site.stateDir}/media-registry/${name}/<media-ref>.json";
        kind = "regular-file";
        owner = principal "user" "root";
        group = principal "group" "root";
        mode = "0600";
        creator = actor "broker" "nixling-priv-broker";
        writers = [ (actor "broker" "nixling-priv-broker") ];
        readers = [ (actor "broker" "nixling-priv-broker") ];
        cleanupPolicy = "never";
        repairPolicy = "broker-fail-closed";
        sensitivity = "secret-adjacent";
        invariants = [ "no-symlink" "root-owned-parent" "broker-opaque-id-only" "scope-authorization-required" ];
      })
    ])
    qemuMediaVms);

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
    paths = basePaths
      ++ hostMutablePaths
      ++ perNormalVmStoragePaths
      ++ perTpmStoragePaths
      ++ perQemuMediaStoragePaths
      ++ nodeWritablePaths
      ++ readinessSocketPaths
      ++ diskInitPaths;
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

in
{
  config = {
    nixling._bundle.storageJson = {
      inherit data;
      installFileName = "storage.json";
      classification = "contractPrivateNonSecret";
      sensitivity = "nonSecret";
    };
  };
}
