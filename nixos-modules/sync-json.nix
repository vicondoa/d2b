{ config, lib, pkgs, ... }:

let
  cfg = config.nixling;
  enabledVms = lib.filterAttrs (_: vm: vm.enable) cfg.vms;

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

  ofdLock = { id, scope, path, owner ? actor "daemon" "nixlingd", scopeClass ? "host", root ? "run" }: {
    inherit id scope;
    pathTemplate = path;
    resourceId = null;
    kind = "ofd";
    ownerProcess = owner;
    allowedHolders = [ owner ];
    inheritancePolicy = "close-on-exec";
    fdPassingPolicy = fdNone;
    acquireOrder = order scopeClass root id id;
    timeoutPolicy = {
      kind = "fail-fast";
      timeoutMs = null;
    };
    stalePolicy = {
      kind = "pidfd-proof-required";
      degradedReason = "lock-owner-ambiguous";
    };
    adoptionPolicy = "reacquire-after-proof";
    degradeScope = if scopeClass == "vm" then "vm" else "host";
    releaseAuthority = owner;
    cloexecRequired = true;
  };

  inProcessLock = vm: {
    id = "lock:op:${vm}";
    scope = "vm:${vm}";
    pathTemplate = null;
    resourceId = "op-lock:${vm}";
    kind = "in-process";
    ownerProcess = actor "daemon" "nixlingd";
    allowedHolders = [ (actor "daemon" "nixlingd") ];
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
    releaseAuthority = actor "daemon" "nixlingd";
    cloexecRequired = true;
  };

  storeSyncLock = vm: ofdLock {
    id = "lock:store-sync:${vm}";
    scope = "vm:${vm}";
    path = "${toString cfg.store.stateDir}/${vm}/store-view/sync.lock";
    owner = actor "broker" "nixling-priv-broker";
    scopeClass = "vm";
    root = "state";
  };

  usbipLock = vm: busid:
    let
      id = lockId "lock:usbip" "${vm}:${busid}";
    in {
      inherit id;
      scope = "vm:${vm}";
      pathTemplate = "/run/nixling/locks/usbip/${busid}";
      resourceId = null;
      kind = "file-record";
      ownerProcess = actor "broker" "nixling-priv-broker";
      allowedHolders = [
        (actor "broker" "nixling-priv-broker")
        (actor "daemon" "nixlingd")
      ];
      inheritancePolicy = "close-on-exec";
      fdPassingPolicy = fdNone;
      acquireOrder = order "vm" "run" id id;
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
      releaseAuthority = actor "broker" "nixling-priv-broker";
      cloexecRequired = true;
    };

  usbipLocks = lib.flatten (lib.mapAttrsToList
    (vm: vmCfg: map (busid: usbipLock vm busid) (vmCfg.usbip.busids or [ ]))
    (lib.filterAttrs (_: vm: vm.enable && vm.usbip.yubikey) cfg.vms));

  data = {
    schemaVersion = "v2";
    locks = [
      (ofdLock {
        id = "lock:daemon";
        scope = "host";
        path = "/run/nixling/daemon.lock";
        owner = actor "daemon" "nixlingd";
        scopeClass = "global";
        root = "run";
      })
    ]
    ++ (lib.mapAttrsToList (vm: _: inProcessLock vm) enabledVms)
    ++ (lib.mapAttrsToList (vm: _: storeSyncLock vm) enabledVms)
    ++ usbipLocks;
  };

  jsonText = builtins.toJSON data;
  jsonFile = pkgs.writeText "nixling-sync.json" jsonText;
in
{
  config = {
    nixling._bundle.syncJson = {
      inherit data jsonText;
      path = "${jsonFile}";
      installFileName = "sync.json";
      classification = "contractPrivateNonSecret";
      sensitivity = "nonSecret";
    };
  };
}
