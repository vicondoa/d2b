#!/usr/bin/env bash
# tests/volume-mounts-eval.sh — pure eval guard for declared VM volumes.
#
# This intentionally avoids a full nixosSystem eval. The canonical volume
# serial / mount / DiskInit predicates live in nixos-modules/lib.nix; module
# callsites use those helpers and the regular flake checks cover integration.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok()   { log "  PASS: $*"; }
fail() { log "  FAIL: $*"; exit 1; }

log "==> tests/volume-mounts-eval.sh"

expr=$(cat <<EOF
let
  lib = rec {
    hasPrefix = prefix: s:
      builtins.substring 0 (builtins.stringLength prefix) s == prefix;
    hasSuffix = suffix: s:
      let
        suffixLen = builtins.stringLength suffix;
        sLen = builtins.stringLength s;
      in
      suffixLen <= sLen
      && builtins.substring (sLen - suffixLen) suffixLen s == suffix;
    removeSuffix = suffix: s:
      if hasSuffix suffix s
      then builtins.substring 0 ((builtins.stringLength s) - (builtins.stringLength suffix)) s
      else s;
    replaceStrings = builtins.replaceStrings;
    optional = cond: value: if cond then [ value ] else [ ];
    filter = builtins.filter;
    stringLength = builtins.stringLength;
    count = pred: list:
      builtins.foldl' (acc: value: if pred value then acc + 1 else acc) 0 list;
    elem = builtins.elem;
    unique = list:
      builtins.foldl'
        (acc: value: if elem value acc then acc else acc ++ [ value ])
        []
        list;
  };
  nl = import ${ROOT}/nixos-modules/lib.nix { inherit lib; };

  varVolume = {
    image = "var.img";
    mountPoint = "/var";
    size = 1024;
    fsType = "ext4";
    serial = null;
  };
  externalVolume = {
    image = "/tmp/external.img";
    mountPoint = "/mnt/external";
    size = 1;
    fsType = "ext4";
  };
  nonExt4Volume = {
    image = "data.img";
    mountPoint = "/data";
    size = 1;
    fsType = "xfs";
  };
  qcowVolume = {
    image = "qcow.img";
    mountPoint = "/qcow";
    size = 1;
    fsType = "ext4";
    imageType = "qcow2";
  };

  issues = nl.volumeSerialIssues [
    { image = "var.img"; }
    { image = "var.img"; }
    { image = "rootfs.img"; }
    { image = "this-name-is-definitely-too-long.img"; }
    { image = "ok.img"; serial = "bad,serial"; }
    { image = "ok2.img"; serial = "bad=serial"; }
    { image = "empty.img"; serial = ""; }
  ];
in {
  serialNullDefaults = nl.volumeSerial varVolume;
  serialSanitizesDelimiters = nl.volumeSerial { image = "bad,name=still.img"; };
  hostPathRelative = nl.volumeHostPath "/var/lib/nixling/vms" "work" varVolume;
  hostPathAbsolute = nl.volumeHostPath "/var/lib/nixling/vms" "work" externalVolume;
  fs = nl.volumeFileSystem varVolume;
  sizeBytes = nl.volumeSizeBytes varVolume;
  diskInitEligible = {
    relativeExt4Raw = nl.volumeDiskInitEligible varVolume;
    absolute = nl.volumeDiskInitEligible externalVolume;
    nonExt4 = nl.volumeDiskInitEligible nonExt4Volume;
    nonRaw = nl.volumeDiskInitEligible qcowVolume;
  };
  inherit issues;
}
EOF
)

if ! json=$(nix-instantiate --eval --strict --json --expr "$expr" 2>&1); then
  fail "eval failed: $json"
fi

expect() {
  local jq_expr="$1" message="$2"
  if printf '%s' "$json" | jq -e "$jq_expr" >/dev/null; then
    ok "$message"
  else
    fail "$message (json: $json)"
  fi
}

expect '.serialNullDefaults == "var"' 'serial = null derives the image-based default'
expect '.serialSanitizesDelimiters == "bad-name-still"' 'derived serials sanitize CH delimiters'
expect '.hostPathRelative == "/var/lib/nixling/vms/work/var.img"' 'relative volume image is rooted under VM state dir'
expect '.hostPathAbsolute == "/tmp/external.img"' 'absolute volume image path is preserved'
expect '.fs.device == "/dev/disk/by-id/virtio-var"' '/var mounts by stable virtio serial'
expect '.fs.fsType == "ext4"' '/var fsType preserved'
expect '.fs.neededForBoot == true' '/var volume is neededForBoot'
expect '(.fs.options | index("x-systemd.after=systemd-modules-load.service")) != null' '/var mount waits for modules'
expect '.sizeBytes == 1073741824' 'volume size converts MiB to bytes'
expect '.diskInitEligible.relativeExt4Raw == true' 'relative raw ext4 volume is DiskInit eligible'
expect '.diskInitEligible.absolute == false' 'absolute external volume is not DiskInit eligible'
expect '.diskInitEligible.nonExt4 == false' 'non-ext4 volume is not DiskInit eligible'
expect '.diskInitEligible.nonRaw == false' 'non-raw volume is not DiskInit eligible'
expect '(.issues.duplicates | index("var")) != null' 'duplicate serials are detected'
expect '(.issues.reserved | index("rootfs")) != null' 'reserved rootfs serial is detected'
expect '(.issues.tooLong | index("this-name-is-definitely-too-long")) != null' 'overlong serial is detected'
expect '(.issues.unsafe | index("bad,serial")) != null' 'explicit comma serial is rejected'
expect '(.issues.unsafe | index("bad=serial")) != null' 'explicit equals serial is rejected'
expect '(.issues.unsafe | index("")) != null' 'empty explicit serial is rejected'

grep -Fq 'serial = nl.volumeSerial volume;' "$ROOT/nixos-modules/processes-json.nix" \
  || fail "processes-json.nix must use shared volumeSerial helper"
grep -Fq 'nl.volumeFileSystem volume' "$ROOT/nixos-modules/vm-guest-base.nix" \
  || fail "vm-guest-base.nix must use shared volumeFileSystem helper"
grep -Fq 'builtins.filter nl.volumeDiskInitEligible microvm.volumes' "$ROOT/nixos-modules/processes-json.nix" \
  || fail "processes-json.nix must gate DiskInit with shared eligibility helper"
ok "module callsites use shared volume helpers"

log "==> tests/volume-mounts-eval.sh OK"
