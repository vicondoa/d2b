{ ... }:

{
  # PID1 owns only the fixed filesystem anchors. Every realm, workload,
  # provider, role, lock, key, audit, and store-view path below these anchors
  # is created or repaired by the broker through an opaque generated ID.
  systemd.tmpfiles.rules = [
    "d /var/lib/d2b 0750 root d2bd -"
    "z /var/lib/d2b 0750 root d2bd -"
    "d /var/cache/d2b 0750 root d2bd -"
    "z /var/cache/d2b 0750 root d2bd -"
    "a+ /run/d2b - - - - m::rwx"
    "a+ /run/d2b - - - - default:m::rwx"
  ];
}
