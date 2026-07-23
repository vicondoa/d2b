{ config, lib, ... }:

let
  cfg = config.d2b;
  d2bLib = import ./lib.nix { inherit lib; };
  enabledVms = lib.filterAttrs (_: vm: vm.enable) cfg.vms;
  qemuMediaVms = d2bLib.qemuMediaVms cfg.vms;

  actor = kind: value: { inherit kind value; };
  lockId = prefix: key: "${prefix}:${builtins.hashString "sha256" key}";

  fdNone = {
    mechanism = "none";
    leaseTransferRecordRequired = false;
  };

  order = scopeClass: root: normalizedPath: id: {
    inherit scopeClass normalizedPath;
    anchoredRoot = root;
    lockId = id;
  };

  ofdLock = { id, scope, path, owner ? actor "daemon" "d2bd", scopeClass ? "host", root ? "run", normalizedPath ? id }: {
    inherit id scope;
    pathTemplate = path;
    resourceId = null;
    kind = "ofd";
    ownerProcess = owner;
    allowedHolders = [ owner ];
    inheritancePolicy = "close-on-exec";
    fdPassingPolicy = fdNone;
    acquireOrder = order scopeClass root normalizedPath id;
    timeoutPolicy = {
      kind = "fail-fast";
      timeoutMs = null;
    };
    stalePolicy = {
      kind = "cutover-only";
      degradedReason = "lock-owner-ambiguous";
    };
    adoptionPolicy = "reacquire-after-proof";
    degradeScope = if scopeClass == "vm" then "vm" else "host";
    releaseAuthority = owner;
    cloexecRequired = true;
  };

  kernelLock = { id, scope, path ? null, resourceId ? null, owner, scopeClass ? "host", root ? "run", normalizedPath ? id, staleKind ? "cutover-only", adoptionPolicy ? "reacquire-after-proof", timeoutKind ? "fail-fast", timeoutMs ? null, degradeScope ? "host" }: {
    inherit id scope;
    pathTemplate = path;
    inherit resourceId;
    kind = "kernel-object";
    ownerProcess = owner;
    allowedHolders = [ owner ];
    inheritancePolicy = "close-on-exec";
    fdPassingPolicy = fdNone;
    acquireOrder = order scopeClass root normalizedPath id;
    timeoutPolicy = {
      kind = timeoutKind;
      inherit timeoutMs;
    };
    stalePolicy = {
      kind = staleKind;
      degradedReason = "lock-owner-ambiguous";
    };
    inherit adoptionPolicy degradeScope;
    releaseAuthority = owner;
    cloexecRequired = true;
  };

  lockRoot = { id, scope, path, owner, root ? "run", normalizedPath, scopeClass ? "host", readers ? [ ] }: {
    inherit id scope;
    pathTemplate = path;
    resourceId = id;
    kind = "kernel-object";
    ownerProcess = owner;
    allowedHolders = [ owner ] ++ readers;
    inheritancePolicy = "not-applicable";
    fdPassingPolicy = fdNone;
    acquireOrder = order scopeClass root normalizedPath id;
    timeoutPolicy = {
      kind = "fail-fast";
      timeoutMs = null;
    };
    stalePolicy = {
      kind = "manual-recovery";
      degradedReason = "lock-owner-ambiguous";
    };
    adoptionPolicy = "not-adoptable";
    degradeScope = if scopeClass == "vm" then "vm" else "host";
    releaseAuthority = owner;
    cloexecRequired = false;
  };

  inProcessLock = vm: {
    id = "lock:op:${vm}";
    scope = "vm:${vm}";
    pathTemplate = null;
    resourceId = "op-lock:${vm}";
    kind = "in-process";
    ownerProcess = actor "daemon" "d2bd";
    allowedHolders = [ (actor "daemon" "d2bd") ];
    inheritancePolicy = "not-applicable";
    fdPassingPolicy = fdNone;
    acquireOrder = order "vm" "daemon" "op-lock:${vm}" "lock:op:${vm}";
    timeoutPolicy = {
      kind = "bounded-wait";
      timeoutMs = 60000;
    };
    stalePolicy = {
      kind = "manual-recovery";
      degradedReason = "lock-owner-ambiguous";
    };
    adoptionPolicy = "quarantine-on-ambiguity";
    degradeScope = "vm";
    releaseAuthority = actor "daemon" "d2bd";
    cloexecRequired = false;
  };

  vmStartLock = vm: ofdLock {
    id = "lock:vm-start:${vm}";
    scope = "vm:${vm}";
    path = "/run/d2b/locks/vm-start-${vm}.lock";
    owner = actor "daemon" "d2bd";
    scopeClass = "vm";
    root = "run";
    normalizedPath = "locks/vm-start-${vm}.lock";
  };

  storeSyncLock = vm: ofdLock {
    id = "lock:store-sync:${vm}";
    scope = "vm:${vm}";
    path = "${toString cfg.store.stateDir}/${vm}/store-view/sync.lock";
    owner = actor "broker" "d2b-priv-broker";
    scopeClass = "vm";
    root = "state";
    normalizedPath = "vms/${vm}/store-view/sync.lock";
  };

  qemuMediaTapGrant = vm: kernelLock {
    id = "lock:qemu-media-tap:${vm}";
    scope = "vm:${vm}";
    resourceId = "tap:${cfg.manifest.${vm}.tap}";
    owner = actor "role" "role:${vm}:qemu-media";
    scopeClass = "vm";
    root = "kernel";
    normalizedPath = "tap/${cfg.manifest.${vm}.tap}";
    degradeScope = "vm";
  };

  usbipLock = vm: busid:
    let
      id = lockId "lock:usbip" "${vm}:${busid}";
    in {
      inherit id;
      scope = "vm:${vm}";
      pathTemplate = "/run/d2b/locks/usbip/${busid}";
      resourceId = null;
      kind = "file-record";
      ownerProcess = actor "broker" "d2b-priv-broker";
      allowedHolders = [
        (actor "broker" "d2b-priv-broker")
        (actor "daemon" "d2bd")
      ];
      inheritancePolicy = "close-on-exec";
      fdPassingPolicy = fdNone;
      acquireOrder = order "host" "run" "run/d2b/locks/usbip/${busid}" id;
      timeoutPolicy = {
        kind = "fail-fast";
        timeoutMs = null;
      };
      stalePolicy = {
        kind = "file-record-owner-match";
        degradedReason = "lock-owner-ambiguous";
      };
      adoptionPolicy = "reacquire-after-proof";
      degradeScope = "vm";
      releaseAuthority = actor "broker" "d2b-priv-broker";
      cloexecRequired = true;
    };

  usbipLocks = lib.flatten (lib.mapAttrsToList
    (vm: vmCfg: map (busid: usbipLock vm busid) (vmCfg.usbip.busids or [ ]))
    (lib.filterAttrs (_: vm: vm.enable && vm.usbip.yubikey) cfg.vms));

  data = {
    schemaVersion = "v2";
    locks = [
      (lockRoot {
        id = "lock-root:run";
        scope = "host";
        path = "/run/d2b";
        owner = actor "nix-module" "tmpfiles";
        normalizedPath = "run/d2b";
        readers = [
          (actor "daemon" "d2bd")
          (actor "broker" "d2b-priv-broker")
        ];
      })
      (lockRoot {
        id = "lock-root:daemon-state";
        scope = "host";
        path = "/run/d2b/state";
        owner = actor "nix-module" "tmpfiles";
        normalizedPath = "run/d2b/state";
        readers = [ (actor "daemon" "d2bd") ];
      })
      (lockRoot {
        id = "lock-root:vm-locks";
        scope = "host";
        path = "/run/d2b/locks";
        owner = actor "nix-module" "tmpfiles";
        normalizedPath = "run/d2b/locks";
        readers = [ (actor "daemon" "d2bd") ];
      })
      (lockRoot {
        id = "lock-root:usbip";
        scope = "host";
        path = "/run/d2b/locks/usbip";
        owner = actor "nix-module" "tmpfiles";
        normalizedPath = "run/d2b/locks/usbip";
        readers = [
          (actor "broker" "d2b-priv-broker")
          (actor "daemon" "d2bd")
        ];
      })
      (ofdLock {
        id = "lock:daemon";
        scope = "host";
        path = "/run/d2b/daemon.lock";
        owner = actor "daemon" "d2bd";
        scopeClass = "global";
        root = "run";
        normalizedPath = "daemon.lock";
      })
    ]
    ++ (lib.mapAttrsToList (vm: _: inProcessLock vm) enabledVms)
    ++ (lib.mapAttrsToList (vm: _: vmStartLock vm) enabledVms)
    ++ (lib.mapAttrsToList (vm: _: storeSyncLock vm) enabledVms)
    ++ (lib.mapAttrsToList (vm: _: qemuMediaTapGrant vm) qemuMediaVms)
    ++ usbipLocks;
  };

in
{
  config = {
    d2b._bundle.syncJson = {
      inherit data;
      installFileName = "sync.json";
      classification = "contractPrivateNonSecret";
      sensitivity = "nonSecret";
    };
  };
}
