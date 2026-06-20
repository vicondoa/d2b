{ config, lib, pkgs, ... }:

let
  privateEtc = source: {
    inherit source;
    mode = "0640";
    user = "root";
    group = if config.nixling.daemonExperimental.enable then "nixlingd" else "root";
  };

  retainedFields = [ "operation" "subject" "scope" "result" ];

  normalizeRow = row:
    (removeAttrs row [ "auditMode" ]) // {
      audit = {
        required = row.auditMode == "yes";
        mode = row.auditMode;
        inherit retainedFields;
      };
      defaultForUnknown = "deny-and-audit";
    };

  publicOperations = map normalizeRow (builtins.fromJSON ''
[
  {
    "operation": "hello",
    "subject": "daemon",
    "scope": "global",
    "allowedGroups": [
      "any-local-client"
    ],
    "destructive": false,
    "secretAccess": "none",
    "brokerRequired": "no",
    "auditMode": "deny-only"
  },
  {
    "operation": "capabilities",
    "subject": "daemon",
    "scope": "global",
    "allowedGroups": [
      "any-local-client"
    ],
    "destructive": false,
    "secretAccess": "none",
    "brokerRequired": "no",
    "auditMode": "deny-only"
  },
  {
    "operation": "auth status",
    "subject": "daemon",
    "scope": "global",
    "allowedGroups": [
      "any-local-client"
    ],
    "destructive": false,
    "secretAccess": "none",
    "brokerRequired": "no",
    "auditMode": "deny-only"
  },
  {
    "operation": "list",
    "subject": "VM/env",
    "scope": "global-or-scoped",
    "allowedGroups": [
      "nixling-launcher",
      "nixling-admin"
    ],
    "destructive": false,
    "secretAccess": "none",
    "brokerRequired": "no",
    "auditMode": "errors"
  },
  {
    "operation": "status",
    "subject": "VM/env",
    "scope": "global-or-scoped",
    "allowedGroups": [
      "nixling-launcher",
      "nixling-admin"
    ],
    "destructive": false,
    "secretAccess": "none",
    "brokerRequired": "no",
    "auditMode": "errors"
  },
  {
    "operation": "status --check-bridges",
    "subject": "VM/env",
    "scope": "global-or-scoped",
    "allowedGroups": [
      "nixling-launcher",
      "nixling-admin"
    ],
    "destructive": false,
    "secretAccess": "none",
    "brokerRequired": "no",
    "auditMode": "errors"
  },
  {
    "operation": "audit",
    "subject": "host/VM",
    "scope": "global",
    "allowedGroups": [
      "nixling-launcher",
      "nixling-admin"
    ],
    "destructive": false,
    "secretAccess": "none",
    "brokerRequired": "no",
    "auditMode": "yes"
  },
  {
    "operation": "audit --human",
    "subject": "host/VM",
    "scope": "global",
    "allowedGroups": [
      "nixling-launcher",
      "nixling-admin"
    ],
    "destructive": false,
    "secretAccess": "none",
    "brokerRequired": "no",
    "auditMode": "yes"
  },
  {
    "operation": "audit --json",
    "subject": "host/VM",
    "scope": "global",
    "allowedGroups": [
      "nixling-launcher",
      "nixling-admin"
    ],
    "destructive": false,
    "secretAccess": "none",
    "brokerRequired": "no",
    "auditMode": "yes"
  },
  {
    "operation": "host check",
    "subject": "host",
    "scope": "global",
    "allowedGroups": [
      "nixling-launcher",
      "nixling-admin"
    ],
    "destructive": false,
    "secretAccess": "none",
    "brokerRequired": "no-mutation",
    "auditMode": "yes"
  },
  {
    "operation": "host doctor --read-only",
    "subject": "host",
    "scope": "global",
    "allowedGroups": [
      "nixling-launcher",
      "nixling-admin"
    ],
    "destructive": false,
    "secretAccess": "none",
    "brokerRequired": "no-mutation",
    "auditMode": "yes"
  },
  {
    "operation": "host prepare",
    "subject": "host",
    "scope": "global",
    "allowedGroups": [
      "nixling-admin"
    ],
    "destructive": false,
    "secretAccess": "none",
    "brokerRequired": "no-mutation",
    "auditMode": "yes"
  },
  {
    "operation": "host prepare --dry-run",
    "subject": "host",
    "scope": "global",
    "allowedGroups": [
      "nixling-admin"
    ],
    "destructive": false,
    "secretAccess": "none",
    "brokerRequired": "no-mutation",
    "auditMode": "yes"
  },
  {
    "operation": "host install",
    "subject": "host",
    "scope": "global",
    "allowedGroups": [
      "nixling-admin"
    ],
    "destructive": false,
    "secretAccess": "none",
    "brokerRequired": "no-mutation",
    "auditMode": "yes"
  },
  {
    "operation": "host destroy --dry-run",
    "subject": "host",
    "scope": "global",
    "allowedGroups": [
      "nixling-admin"
    ],
    "destructive": false,
    "secretAccess": "none",
    "brokerRequired": "no-mutation",
    "auditMode": "yes"
  },
  {
    "operation": "host prepare --apply",
    "subject": "host",
    "scope": "global",
    "allowedGroups": [
      "nixling-admin"
    ],
    "destructive": true,
    "secretAccess": "possible-paths-only",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "host reconcile-otel-acls --apply",
    "subject": "host/observability",
    "scope": "global",
    "allowedGroups": [
      "nixling-admin"
    ],
    "destructive": true,
    "secretAccess": "metadata-only",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "host destroy --apply",
    "subject": "host",
    "scope": "global",
    "allowedGroups": [
      "nixling-admin"
    ],
    "destructive": true,
    "secretAccess": "possible-paths-only",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "host migrate-storage --apply",
    "subject": "host/storage",
    "scope": "global",
    "allowedGroups": [
      "nixling-admin"
    ],
    "destructive": true,
    "secretAccess": "possible-paths-only",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "up",
    "subject": "VM/env",
    "scope": "per-VM/per-env",
    "allowedGroups": [
      "nixling-launcher",
      "nixling-admin"
    ],
    "destructive": false,
    "secretAccess": "none",
    "brokerRequired": "conditional",
    "auditMode": "yes"
  },
  {
    "operation": "down",
    "subject": "VM/env",
    "scope": "per-VM/per-env",
    "allowedGroups": [
      "nixling-launcher",
      "nixling-admin"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "conditional",
    "auditMode": "yes"
  },
  {
    "operation": "restart",
    "subject": "VM/env",
    "scope": "per-VM/per-env",
    "allowedGroups": [
      "nixling-launcher",
      "nixling-admin"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "conditional",
    "auditMode": "yes"
  },
  {
    "operation": "console",
    "subject": "VM",
    "scope": "per-VM",
    "allowedGroups": [
      "nixling-launcher",
      "nixling-admin"
    ],
    "destructive": false,
    "secretAccess": "none",
    "brokerRequired": "no",
    "auditMode": "yes"
  },
  {
    "operation": "config",
    "subject": "VM",
    "scope": "per-VM",
    "allowedGroups": [
      "nixling-launcher",
      "nixling-admin"
    ],
    "destructive": false,
    "secretAccess": "none",
    "brokerRequired": "no",
    "auditMode": "yes"
  },
  {
    "operation": "build",
    "subject": "VM",
    "scope": "per-VM",
    "allowedGroups": [
      "nixling-launcher",
      "nixling-admin"
    ],
    "destructive": false,
    "secretAccess": "none",
    "brokerRequired": "conditional",
    "auditMode": "yes"
  },
  {
    "operation": "generations",
    "subject": "VM",
    "scope": "per-VM",
    "allowedGroups": [
      "nixling-launcher",
      "nixling-admin"
    ],
    "destructive": false,
    "secretAccess": "none",
    "brokerRequired": "no",
    "auditMode": "yes"
  },
  {
    "operation": "exec",
    "subject": "VM/process",
    "scope": "per-VM",
    "allowedGroups": [
      "nixling-admin"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "no",
    "auditMode": "yes"
  },
  {
    "operation": "switch",
    "subject": "VM",
    "scope": "per-VM",
    "allowedGroups": [
      "nixling-admin"
    ],
    "destructive": true,
    "secretAccess": "metadata-only",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "boot",
    "subject": "VM",
    "scope": "per-VM",
    "allowedGroups": [
      "nixling-admin"
    ],
    "destructive": true,
    "secretAccess": "metadata-only",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "test",
    "subject": "VM",
    "scope": "per-VM",
    "allowedGroups": [
      "nixling-admin"
    ],
    "destructive": true,
    "secretAccess": "metadata-only",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "rollback",
    "subject": "VM",
    "scope": "per-VM",
    "allowedGroups": [
      "nixling-admin"
    ],
    "destructive": true,
    "secretAccess": "metadata-only",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "gc",
    "subject": "VM/global",
    "scope": "per-VM/global",
    "allowedGroups": [
      "nixling-admin"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "store verify",
    "subject": "store/VM",
    "scope": "per-VM",
    "allowedGroups": [
      "nixling-admin"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "keys list",
    "subject": "key",
    "scope": "per-VM",
    "allowedGroups": [
      "nixling-admin"
    ],
    "destructive": false,
    "secretAccess": "public-key-only",
    "brokerRequired": "no",
    "auditMode": "yes"
  },
  {
    "operation": "keys show",
    "subject": "key",
    "scope": "per-VM",
    "allowedGroups": [
      "nixling-admin"
    ],
    "destructive": false,
    "secretAccess": "public-key-only",
    "brokerRequired": "no",
    "auditMode": "yes"
  },
  {
    "operation": "keys rotate",
    "subject": "key",
    "scope": "per-VM",
    "allowedGroups": [
      "nixling-admin"
    ],
    "destructive": true,
    "secretAccess": "read-write",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "trust",
    "subject": "key/known-host",
    "scope": "per-VM",
    "allowedGroups": [
      "nixling-admin"
    ],
    "destructive": true,
    "secretAccess": "host-key-metadata",
    "brokerRequired": "conditional",
    "auditMode": "yes"
  },
  {
    "operation": "rotate-known-host",
    "subject": "key/known-host",
    "scope": "per-VM",
    "allowedGroups": [
      "nixling-admin"
    ],
    "destructive": true,
    "secretAccess": "host-key-metadata",
    "brokerRequired": "conditional",
    "auditMode": "yes"
  },
  {
    "operation": "audio",
    "subject": "VM/audio",
    "scope": "per-VM",
    "allowedGroups": [
      "nixling-launcher",
      "nixling-admin"
    ],
    "destructive": false,
    "secretAccess": "none",
    "brokerRequired": "no",
    "auditMode": "errors"
  },
  {
    "operation": "audio status",
    "subject": "VM/audio",
    "scope": "per-VM",
    "allowedGroups": [
      "nixling-launcher",
      "nixling-admin"
    ],
    "destructive": false,
    "secretAccess": "none",
    "brokerRequired": "no",
    "auditMode": "errors"
  },
  {
    "operation": "audio mic",
    "subject": "VM/audio",
    "scope": "per-VM",
    "allowedGroups": [
      "nixling-launcher",
      "nixling-admin"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "audio speaker",
    "subject": "VM/audio",
    "scope": "per-VM",
    "allowedGroups": [
      "nixling-launcher",
      "nixling-admin"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "audio on",
    "subject": "VM/audio",
    "scope": "per-VM",
    "allowedGroups": [
      "nixling-launcher",
      "nixling-admin"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "audio off",
    "subject": "VM/audio",
    "scope": "per-VM",
    "allowedGroups": [
      "nixling-launcher",
      "nixling-admin"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "usb",
    "subject": "VM/USB busid",
    "scope": "per-VM/per-env",
    "allowedGroups": [
      "nixling-launcher",
      "nixling-admin"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "usb attach",
    "subject": "VM/USB busid",
    "scope": "per-VM/per-env/per-busid",
    "allowedGroups": [
      "nixling-launcher",
      "nixling-admin"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "usb enroll",
    "subject": "VM/USB media ref",
    "scope": "per-VM/per-media-ref",
    "allowedGroups": [
      "nixling-admin"
    ],
    "destructive": true,
    "secretAccess": "redacted-only",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "usb detach",
    "subject": "VM/USB busid",
    "scope": "per-VM/per-env/per-busid",
    "allowedGroups": [
      "nixling-launcher",
      "nixling-admin"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "usb probe",
    "subject": "VM/USB busid",
    "scope": "global",
    "allowedGroups": [
      "nixling-launcher",
      "nixling-admin"
    ],
    "destructive": false,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "debug bundle",
    "subject": "diagnostics",
    "scope": "scoped",
    "allowedGroups": [
      "nixling-admin"
    ],
    "destructive": false,
    "secretAccess": "redacted-only",
    "brokerRequired": "no-mutation",
    "auditMode": "yes"
  },
  {
    "operation": "migrate",
    "subject": "host/state",
    "scope": "global",
    "allowedGroups": [
      "nixling-admin"
    ],
    "destructive": true,
    "secretAccess": "metadata-only",
    "brokerRequired": "yes",
    "auditMode": "yes"
  }
]
'');

  brokerOperations = map normalizeRow (builtins.fromJSON ''
[
  {
    "operation": "Hello",
    "subject": "handshake",
    "scope": "global",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": false,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "ValidateBundle",
    "subject": "bundle",
    "scope": "global",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": false,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "RunHostInstall",
    "subject": "installer",
    "scope": "global",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "RunMigrate",
    "subject": "installer",
    "scope": "global",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "RunActivation",
    "subject": "VM",
    "scope": "per-VM",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "metadata-only",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "RunGc",
    "subject": "VM/global",
    "scope": "per-VM/global",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "RunKeysRotate",
    "subject": "key",
    "scope": "per-VM",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "read-write",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "RunHostKeyTrust",
    "subject": "key/known-host",
    "scope": "per-VM",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "host-key-metadata",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "RunRotateKnownHost",
    "subject": "key/known-host",
    "scope": "per-VM",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "host-key-metadata",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "PrepareRuntimeDir",
    "subject": "fs",
    "scope": "global/per-VM",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "PrepareStateDir",
    "subject": "fs",
    "scope": "global/per-VM",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "PrepareSwtpmDir",
    "subject": "fs",
    "scope": "per-VM",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "metadata-only",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "CreateOrReconcileUsersGroups",
    "subject": "account",
    "scope": "global/per-role",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "DelegateCgroupV2",
    "subject": "cgroup",
    "scope": "global/per-VM/role",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": false,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "OpenCgroupDir",
    "subject": "cgroup",
    "scope": "global/per-VM/role",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": false,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "OpenKvm",
    "subject": "device",
    "scope": "per-role",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": false,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "OpenPidfd",
    "subject": "pidfd",
    "scope": "per-VM/role",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": false,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "OpenVhostNet",
    "subject": "device",
    "scope": "per-role",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": false,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "OpenFuse",
    "subject": "device",
    "scope": "per-role",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": false,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "OpenDevice",
    "subject": "device",
    "scope": "per-role",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": false,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "CreateTapFd",
    "subject": "network",
    "scope": "per-env/VM/TAP",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "CreatePersistentTap",
    "subject": "network",
    "scope": "per-env/VM/TAP",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "SetBridgePortFlags",
    "subject": "network",
    "scope": "per-env/VM/TAP",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "ApplyNftables",
    "subject": "network-host",
    "scope": "global/per-env",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "ApplyRoute",
    "subject": "network-host",
    "scope": "global/per-env",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "ApplySysctl",
    "subject": "network-host",
    "scope": "global/per-env",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "ApplyNmUnmanaged",
    "subject": "network-host",
    "scope": "global/per-env",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "UpdateHostsFile",
    "subject": "name-resolution",
    "scope": "global",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "BindUnixSocket",
    "subject": "socket",
    "scope": "per-VM/role",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "SetSocketAcl",
    "subject": "socket",
    "scope": "per-VM/role",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "SetupMountNamespace",
    "subject": "mount/store",
    "scope": "per-VM/role",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "PrepareStoreView",
    "subject": "mount/store",
    "scope": "per-VM/role",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "LaunchMinijailChild",
    "subject": "process",
    "scope": "per-VM/role",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "ReadSecretById",
    "subject": "secret/key",
    "scope": "per-VM/key",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": false,
    "secretAccess": "read-write",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "GuestControlSign",
    "subject": "guest-control token",
    "scope": "per-VM",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": false,
    "secretAccess": "redacted-only",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "InjectSecretById",
    "subject": "secret/key",
    "scope": "per-VM/key",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": false,
    "secretAccess": "read-write",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "RotateSecretById",
    "subject": "secret/key",
    "scope": "per-VM/key",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "read-write",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "UsbipBind",
    "subject": "USBIP",
    "scope": "per-busid/env",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "UsbipUnbind",
    "subject": "USBIP",
    "scope": "per-busid/env",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "UsbipProxyReconcile",
    "subject": "USBIP",
    "scope": "per-busid/env",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "ModprobeIfAllowed",
    "subject": "kernel-module",
    "scope": "global/feature",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "PauseBroker",
    "subject": "broker-admin",
    "scope": "global",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "metadata-only",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "ResumeBroker",
    "subject": "broker-admin",
    "scope": "global",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "metadata-only",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "ExportBrokerAudit",
    "subject": "broker-admin",
    "scope": "global",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": false,
    "secretAccess": "metadata-only",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "UsbipBindFirewallRule",
    "subject": "USBIP firewall",
    "scope": "per-busid",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": false,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "QemuMediaEnroll",
    "subject": "qemu-media registry",
    "scope": "per-VM/per-media-ref",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "redacted-only",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "QemuMediaAttach",
    "subject": "qemu-media hotplug",
    "scope": "per-VM/per-media-ref",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "redacted-only",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "QemuMediaRefreshRegistry",
    "subject": "qemu-media redacted registry",
    "scope": "host",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": false,
    "secretAccess": "redacted-only",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "ReconcileStorageScope",
    "subject": "fs/storage-contract",
    "scope": "global/per-VM/per-role",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "metadata-only",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "ValidateLockSpec",
    "subject": "lock/sync-contract",
    "scope": "global/per-VM/per-role",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": false,
    "secretAccess": "metadata-only",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "QemuMediaBoot",
    "subject": "qemu-media boot media",
    "scope": "per-VM/per-media-ref",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "redacted-only",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "QemuMediaDetach",
    "subject": "qemu-media hotplug",
    "scope": "per-VM/per-media-ref",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "redacted-only",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "SignalRunner",
    "subject": "runner",
    "scope": "per-VM",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "DeregisterRunnerPidfd",
    "subject": "runner",
    "scope": "per-VM",
    "allowedGroups": [
      "nixling-launcher",
      "nixling-admin"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "PollChildReaped",
    "subject": "runner",
    "scope": "global",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": false,
    "secretAccess": "metadata-only",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "StoreSync",
    "subject": "store",
    "scope": "per-VM",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "StoreVerify",
    "subject": "store",
    "scope": "per-VM",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "SeedDnsmasqLease",
    "subject": "network",
    "scope": "per-VM/env",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "BindMountFromHardlinkFarm",
    "subject": "mount/store",
    "scope": "per-VM",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "OwnershipMatrixCheck",
    "subject": "host",
    "scope": "per-VM",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": false,
    "secretAccess": "metadata-only",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "SshHostKeyPreflight",
    "subject": "ssh-host-key",
    "scope": "per-VM",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": false,
    "secretAccess": "metadata-only",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "DiskInit",
    "subject": "disk",
    "scope": "per-VM",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  },
  {
    "operation": "SpawnRunner",
    "subject": "vm-runner",
    "scope": "per-VM/role",
    "allowedGroups": [
      "nixlingd"
    ],
    "destructive": true,
    "secretAccess": "none",
    "brokerRequired": "yes",
    "auditMode": "yes"
  }
]
'');

  data = {
    schemaVersion = "v2";
    inherit publicOperations brokerOperations;
  };

  jsonText = builtins.toJSON data;
  jsonFile = pkgs.writeText "nixling-privileges.json" jsonText;
in
{
  options.nixling._bundle.privilegesJson = lib.mkOption {
    type = lib.types.unspecified;
    readOnly = true;
    internal = true;
    description = "Internal schema-v1 privileges.json artifact metadata.";
  };

  config = {
    nixling._bundle.privilegesJson = {
      inherit data jsonText;
      path = "${jsonFile}";
    };
    environment.etc."nixling/privileges.json" = privateEtc jsonFile;
  };
}
