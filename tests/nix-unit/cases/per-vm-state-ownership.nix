# nix-unit cases migrated from tests/per-vm-state-ownership-eval.sh.
#
# Asserts the canonical per-VM state ownership matrix from
# nixos-modules/options-ownership-matrix.nix and the source-level store-sync
# guardrails that the bash gate also carried. The matrix checks mirror the
# original host eval fixture and cover structure, required path presence,
# per-path owner/group/mode posture, hardlink-farm recursion carve-outs, and
# the signed store-view layout.
#
# Spec correction (existing code is canon): the retired gate's prose used the
# shorthand `nixling:<group>` for several host-created entries, but the
# committed matrix owner is `nixlingd` (for example
# `nixlingd:nixling 0600` for store-view/sync.lock and
# `nixlingd:users 0644` for the live marker). These cases assert the current
# matrix values rather than the stale prose shorthand.
{ mkEval, lib, flakeRoot, ... }:

let
  configMod = { lib, ... }: {
    boot.loader.grub.enable = false;
    boot.loader.systemd-boot.enable = false;
    boot.initrd.includeDefaultModules = false;
    fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
    environment.etc."machine-id".text = "00000000000000000000000000000000";
    system.stateVersion = "25.11";
    users.users.alice = { isNormalUser = true; uid = 1000; };
    nixling.site = {
      waylandUser = "alice";
      launcherUsers = [ "alice" ];
    };
  };

  matrix = (mkEval [ configMod ]).config.nixling.daemon.perVmStateOwnershipMatrix;
  octalRe = "^[0-7]{3,4}$";

  hasPath = path: builtins.any (e: e.path == path) matrix;
  entry = path:
    let matches = builtins.filter (e: e.path == path) matrix;
    in if matches == [ ] then throw "per-vm-state-ownership: missing matrix path ${path}"
       else builtins.head matches;
  hasPathCase = path: { expr = hasPath path; expected = true; };
  fieldCase = path: field: expected: {
    expr = (entry path).${field};
    inherit expected;
  };

  linesOf = rel: lib.splitString "\n" (builtins.readFile (flakeRoot + rel));
  storeLines = linesOf "/nixos-modules/store.nix";
  ownershipPreflightLines = linesOf "/packages/nixlingd/src/ownership_preflight.rs";
  hasLine = lines: needle: lib.any (line: lib.hasInfix needle line) lines;

  filesUnder = dir:
    let
      entries = builtins.readDir dir;
    in lib.concatMap
      (name:
        let
          kind = entries.${name};
          path = dir + "/${name}";
        in
          if kind == "directory" then filesUnder path
          else if kind == "regular" || kind == "symlink" then [ path ]
          else [ ])
      (lib.attrNames entries);
  nixosModuleLines = lib.concatMap
    (path: lib.splitString "\n" (builtins.readFile path))
    (filesUnder (flakeRoot + "/nixos-modules"));
  storeStoreMeta2775Line = line:
       lib.hasInfix "store store-meta" line
    && (lib.hasInfix "--mode 2775" line || lib.hasInfix "chmod 2775" line);

  indexedLines = lines:
    builtins.genList
      (i: { inherit i; line = builtins.elemAt lines i; })
      (builtins.length lines);
  ownershipLen = builtins.length ownershipPreflightLines;
  storePathIndices = map (x: x.i)
    (builtins.filter (x: lib.hasInfix "path: \"store" x.line)
      (indexedLines ownershipPreflightLines));
  storePathWindows = lib.concatMap
    (i:
      builtins.genList
        (offset: builtins.elemAt ownershipPreflightLines (i + offset))
        (if ownershipLen - i < 6 then ownershipLen - i else 6))
    storePathIndices;
in
{
  # ---- Structural matrix invariants ----
  "per-vm-state-ownership/matrix-count-at-least-five" = {
    expr = builtins.length matrix >= 5;
    expected = true;
  };
  "per-vm-state-ownership/all-paths-strings" = {
    expr = builtins.all (e: (e ? path) && builtins.isString e.path) matrix;
    expected = true;
  };
  "per-vm-state-ownership/all-owners-strings" = {
    expr = builtins.all (e: (e ? owner) && builtins.isString e.owner) matrix;
    expected = true;
  };
  "per-vm-state-ownership/all-groups-strings" = {
    expr = builtins.all (e: (e ? group) && builtins.isString e.group) matrix;
    expected = true;
  };
  "per-vm-state-ownership/all-modes-octal" = {
    expr = builtins.all
      (e: (e ? mode) && builtins.isString e.mode && builtins.match octalRe e.mode != null)
      matrix;
    expected = true;
  };
  "per-vm-state-ownership/all-kinds-known" = {
    expr = builtins.all (e: (e ? kind) && (e.kind == "dir" || e.kind == "file")) matrix;
    expected = true;
  };
  "per-vm-state-ownership/all-required-bools" = {
    expr = builtins.all (e: (e ? required) && builtins.isBool e.required) matrix;
    expected = true;
  };
  "per-vm-state-ownership/all-recursive-bools" = {
    expr = builtins.all (e: (e ? recursive) && builtins.isBool e.recursive) matrix;
    expected = true;
  };
  "per-vm-state-ownership/all-descriptions-nonempty" = {
    expr = builtins.all
      (e: (e ? description) && builtins.isString e.description && e.description != "")
      matrix;
    expected = true;
  };

  # ---- Canonical entry presence ----
  "per-vm-state-ownership/root-exists" = hasPathCase ".";
  "per-vm-state-ownership/store-exists" = hasPathCase "store";
  "per-vm-state-ownership/swtpm-exists" = hasPathCase "swtpm";
  "per-vm-state-ownership/store-view-meta-exists" = hasPathCase "store-view/meta";
  "per-vm-state-ownership/store-view-generations-absent" = {
    expr = hasPath "store-view/generations";
    expected = false;
  };

  # ---- Legacy store hardlink-farm carve-out ----
  "per-vm-state-ownership/store-owner" = fieldCase "store" "owner" "nixlingd";
  "per-vm-state-ownership/store-group" = fieldCase "store" "group" "users";
  "per-vm-state-ownership/store-mode" = fieldCase "store" "mode" "0755";
  "per-vm-state-ownership/store-recursive-false" = fieldCase "store" "recursive" false;
  "per-vm-state-ownership/store-required-false" = fieldCase "store" "required" false;

  # ---- Per-VM swtpm runner ownership ----
  "per-vm-state-ownership/swtpm-owner" = fieldCase "swtpm" "owner" "nixling-<vm>-swtpm";
  "per-vm-state-ownership/swtpm-group" = fieldCase "swtpm" "group" "nixling-<vm>-swtpm";
  "per-vm-state-ownership/swtpm-mode" = fieldCase "swtpm" "mode" "0700";
  "per-vm-state-ownership/swtpm-owner-templated" = {
    expr = builtins.match ".*<vm>.*" (entry "swtpm").owner != null;
    expected = true;
  };
  "per-vm-state-ownership/swtpm-group-templated" = {
    expr = builtins.match ".*<vm>.*" (entry "swtpm").group != null;
    expected = true;
  };

  # ---- Signed store-view layout ----
  "per-vm-state-ownership/sync-lock-exists" = hasPathCase "store-view/sync.lock";
  "per-vm-state-ownership/sync-lock-owner" = fieldCase "store-view/sync.lock" "owner" "nixlingd";
  "per-vm-state-ownership/sync-lock-group" = fieldCase "store-view/sync.lock" "group" "nixling";
  "per-vm-state-ownership/sync-lock-mode" = fieldCase "store-view/sync.lock" "mode" "0600";
  "per-vm-state-ownership/sync-lock-kind" = fieldCase "store-view/sync.lock" "kind" "file";

  "per-vm-state-ownership/meta-owner" = fieldCase "store-view/meta" "owner" "nixlingd";
  "per-vm-state-ownership/meta-group" = fieldCase "store-view/meta" "group" "users";
  "per-vm-state-ownership/meta-mode" = fieldCase "store-view/meta" "mode" "0755";
  "per-vm-state-ownership/meta-kind" = fieldCase "store-view/meta" "kind" "dir";

  "per-vm-state-ownership/state-exists" = hasPathCase "store-view/state";
  "per-vm-state-ownership/state-owner" = fieldCase "store-view/state" "owner" "nixlingd";
  "per-vm-state-ownership/state-group" = fieldCase "store-view/state" "group" "nixling";
  "per-vm-state-ownership/state-mode" = fieldCase "store-view/state" "mode" "0750";

  "per-vm-state-ownership/gcroots-exists" = hasPathCase "store-view/gcroots";
  "per-vm-state-ownership/gcroots-owner" = fieldCase "store-view/gcroots" "owner" "nixlingd";
  "per-vm-state-ownership/gcroots-group" = fieldCase "store-view/gcroots" "group" "nixling";
  "per-vm-state-ownership/gcroots-mode" = fieldCase "store-view/gcroots" "mode" "0750";

  "per-vm-state-ownership/live-marker-exists" = hasPathCase "store-view/live/.nixling-marker-<vm>";
  "per-vm-state-ownership/live-marker-owner" = fieldCase "store-view/live/.nixling-marker-<vm>" "owner" "nixlingd";
  "per-vm-state-ownership/live-marker-group" = fieldCase "store-view/live/.nixling-marker-<vm>" "group" "users";
  "per-vm-state-ownership/live-marker-mode" = fieldCase "store-view/live/.nixling-marker-<vm>" "mode" "0644";
  "per-vm-state-ownership/live-marker-kind" = fieldCase "store-view/live/.nixling-marker-<vm>" "kind" "file";
  "per-vm-state-ownership/live-marker-required-false" = fieldCase "store-view/live/.nixling-marker-<vm>" "required" false;

  # ---- Store-sync/source guardrails from the retired bash gate ----
  "per-vm-state-ownership/store-sync-chown-meta-dirs" = {
    expr = hasLine storeLines ''find "$META_DIR" -type d -exec chown nixlingd:users {} +'';
    expected = true;
  };
  "per-vm-state-ownership/store-sync-chmod-meta-dirs" = {
    expr = hasLine storeLines ''find "$META_DIR" -type d -exec chmod 0755 {} +'';
    expected = true;
  };
  "per-vm-state-ownership/store-sync-no-root-kvm" = {
    expr = hasLine storeLines "chown root:kvm";
    expected = false;
  };
  "per-vm-state-ownership/store-sync-no-chmod-2775" = {
    expr = hasLine storeLines "chmod 2775";
    expected = false;
  };
  "per-vm-state-ownership/nixos-modules-no-store-store-meta-2775-line" = {
    expr = builtins.any storeStoreMeta2775Line nixosModuleLines;
    expected = false;
  };
  "per-vm-state-ownership/daemon-preflight-no-store-2775-mode" = {
    expr = hasLine storePathWindows "mode: 0o2775";
    expected = false;
  };
}
