# Host ACLs for the observability relay, bridge, and CH exporter users.
{ config, lib, pkgs, ... }:

let
  cfg = config.nixling;
  obsCfg = cfg.observability;
  vmStateDir = name: "${cfg.store.stateDir}/${name}";
  apiSocketPath = name: "${vmStateDir name}/${name}.sock";
  obsOtlpPort = 14317;
  vsockSocketForPort = socketPath: port: "${socketPath}_${toString port}";
  relaySocketPath = name: vsockSocketForPort "${vmStateDir name}/vsock.sock" obsOtlpPort;
  # v0.2.0+: the bridge speaks CH's textual protocol to the stack
  # VM's BASE vsock UDS (vsock.sock), not the per-port file (which
  # doesn't exist for the host->guest direction). Grant access to
  # the base socket too.
  baseVsockSocket = name: "${vmStateDir name}/vsock.sock";

  workloadObsVmNames =
    lib.attrNames
      (lib.filterAttrs (_: vm: vm.enable && vm.observability.enable) cfg.vms);

  enabledVmNames = lib.attrNames (lib.filterAttrs (_: vm: vm.enable) cfg.vms);

  obsVmEnabled =
    obsCfg.enable
    && cfg.vms ? ${obsCfg.vmName}
    && cfg.vms.${obsCfg.vmName}.enable;

  chExporterEnabled = obsCfg.enable && obsCfg.ch.exporter.enable;

  relayVmNames =
    if obsVmEnabled
    then lib.filter (name: name != obsCfg.vmName) workloadObsVmNames
    else [ ];

  relayEndpointVmNames =
    lib.unique (relayVmNames ++ lib.optional obsVmEnabled obsCfg.vmName);

  relayEndpointStateDirs = map vmStateDir relayEndpointVmNames;
  # Workload VMs: relay LISTENs at <vm>/vsock.sock_14317 (guest->host).
  # Stack VM: relay speaks CH textual protocol on <vm>/vsock.sock
  # (host->guest). Two different sockets per endpoint type.
  relayEndpointSockets =
    (map relaySocketPath relayVmNames)
    ++ lib.optional obsVmEnabled (baseVsockSocket obsCfg.vmName);

  bridgeStateDirs = lib.optional obsVmEnabled (vmStateDir obsCfg.vmName);
  # v0.2.0+: bridge speaks CH textual protocol on the BASE UDS
  # (vsock.sock), so grant access there instead of the (non-
  # existent) per-port file.
  bridgeSockets = lib.optional obsVmEnabled (baseVsockSocket obsCfg.vmName);

  chExporterStateDirs = map vmStateDir enabledVmNames;
  chExporterSockets = map apiSocketPath enabledVmNames;

  shellArray = values: lib.concatStringsSep " " (map lib.escapeShellArg values);

  otelAclRefresh = pkgs.writeShellApplication {
    name = "nixling-otel-acl-refresh";
    runtimeInputs = with pkgs; [ acl coreutils gnugrep ];
    # SC2034: the six `*_keep_{dirs,sockets}` arrays below are passed
    # by NAME to `refresh_acl_set` and dereferenced via bash namerefs
    # (`local -n keep_dirs=$dir_keep_name`); shellcheck does not
    # understand namerefs and flags them as unused. They ARE used.
    excludeShellChecks = [ "SC2034" ];
    text = ''
      set -eu

      state_root=${lib.escapeShellArg cfg.store.stateDir}
      relay_keep_dirs=( ${shellArray relayEndpointStateDirs} )
      relay_keep_sockets=( ${shellArray relayEndpointSockets} )
      bridge_keep_dirs=( ${shellArray bridgeStateDirs} )
      bridge_keep_sockets=( ${shellArray bridgeSockets} )
      ch_keep_dirs=( ${shellArray chExporterStateDirs} )
      ch_keep_sockets=( ${shellArray chExporterSockets} )

      setfacl_cmd=( setfacl )
      if setfacl --help 2>&1 | grep -q -- '--physical'; then
        setfacl_cmd+=( --physical )
      fi

      run_setfacl() {
        "''${setfacl_cmd[@]}" "$@"
      }

      has_path() {
        local needle="$1"
        shift
        local path
        for path in "$@"; do
          if [ "$path" = "$needle" ]; then
            return 0
          fi
        done
        return 1
      }

      canonical_path() {
        local path="$1"
        readlink -m -- "$path"
      }

      path_under_state_root() {
        local path="$1"
        local resolved=""
        resolved="$(canonical_path "$path")" || return 1
        case "$resolved" in
          "$state_root"/*) return 0 ;;
          *) return 1 ;;
        esac
      }

      is_safe_dir() {
        local dir="$1"
        path_under_state_root "$dir" && [ ! -L "$dir" ] && [ -d "$dir" ]
      }

      is_safe_socket() {
        local path="$1"
        path_under_state_root "$path" && [ ! -L "$path" ] && [ -S "$path" ]
      }

      remove_dir_acl_if_present() {
        local entity="$1"
        local dir="$2"
        if is_safe_dir "$dir"; then
          run_setfacl -d -x "$entity" "$dir" || true
          run_setfacl -x "$entity" "$dir" || true
        fi
      }

      remove_socket_acl_if_present() {
        local entity="$1"
        local path="$2"
        if is_safe_socket "$path"; then
          run_setfacl -x "$entity" "$path" || true
        fi
      }

      grant_dir_acl() {
        local entity="$1"
        local dir="$2"
        local mode="''${3:---x}"
        if is_safe_dir "$dir"; then
          run_setfacl -d -x "$entity" "$dir" || true
          run_setfacl -m "$entity:$mode" "$dir" || true
        fi
      }

      grant_socket_acl() {
        local entity="$1"
        local path="$2"
        if is_safe_socket "$path"; then
          run_setfacl -m "$entity:rw" "$path" || true
        fi
      }

      revoke_if_stale() {
        local entity="$1"
        local kind="$2"
        local path="$3"
        local keep_name="$4"
        local -n keep_ref="$keep_name"

        if has_path "$path" "''${keep_ref[@]}"; then
          return 0
        fi

        case "$kind" in
          dir) remove_dir_acl_if_present "$entity" "$path" ;;
          socket) remove_socket_acl_if_present "$entity" "$path" ;;
        esac
      }

      refresh_acl_set() {
        local entity="$1"
        local dir_keep_name="$2"
        local socket_keep_name="$3"
        local socket_template="$4"
        local dir_mode="''${5:---x}"
        local -n keep_dirs="$dir_keep_name"
        local -n keep_sockets="$socket_keep_name"
        local dir
        local vm_name
        local candidate_socket

        if [ -d "$state_root" ]; then
          for dir in "$state_root"/*; do
            [ -e "$dir" ] || continue
            if ! is_safe_dir "$dir"; then
              continue
            fi

            vm_name="''${dir##*/}"
            candidate_socket="$dir/''${socket_template//%VM%/$vm_name}"

            revoke_if_stale "$entity" dir "$dir" "$dir_keep_name"
            revoke_if_stale "$entity" socket "$candidate_socket" "$socket_keep_name"
          done
        fi

        for dir in "''${keep_dirs[@]}"; do
          grant_dir_acl "$entity" "$dir" "$dir_mode"
        done

        for path in "''${keep_sockets[@]}"; do
          grant_socket_acl "$entity" "$path"
        done
      }

      # v0.2.0+ pre-pass: the OLD pre-v0.2.0 ACL grants on the
      # base vsock.sock LISTENers were defensive (the bridge used
      # the textual protocol; the relay didn't speak to the base
      # at all). v0.2.0 needs the bridge AND the relay to access
      # the BASE UDS for the stack VM, so the pre-pass below now
      # only revokes from VMs that are NOT the obs stack.
      if [ -d "$state_root" ]; then
        for dir in "$state_root"/*; do
          [ -e "$dir" ] || continue
          if ! is_safe_dir "$dir"; then
            continue
          fi
          if [ "''${dir##*/}" = ${lib.escapeShellArg obsCfg.vmName} ]; then
            continue
          fi
          remove_socket_acl_if_present "g:nixling-otel-relay" "$dir/vsock.sock"
          remove_socket_acl_if_present "g:nixling-otel-bridge" "$dir/vsock.sock"
        done
      fi

      # v0.2.0+: the per-VM relay does UNIX-LISTEN at
      # <vm>/vsock.sock_<obsOtlpPort> (CH proxies the workload
      # guest's VSOCK-CONNECT:2:14317 to this LISTEN). Bind requires
      # write+exec on the parent dir, not just traverse — grant rwx
      # to nixling-otel-relay. The relay ALSO needs read+write on
      # the obs stack VM's BASE vsock.sock for the CH textual
      # protocol. The bridge has the same need.
      refresh_acl_set "g:nixling-otel-relay" relay_keep_dirs relay_keep_sockets "vsock.sock_${toString obsOtlpPort}" rwx
      refresh_acl_set "g:nixling-otel-bridge" bridge_keep_dirs bridge_keep_sockets "vsock.sock"
      refresh_acl_set "g:nixling-ch-exporter" ch_keep_dirs ch_keep_sockets "%VM%.sock"
    '';
  };
  otelAclRefreshBin = "${otelAclRefresh}/bin/nixling-otel-acl-refresh";

  lockedSystemUser = description: group: {
    isSystemUser = true;
    inherit group description;
    home = "/var/empty";
    createHome = false;
    hashedPassword = "!";
    shell = pkgs.shadow + "/bin/nologin";
  };
in
lib.mkMerge [
  (lib.mkIf (relayVmNames != [ ]) {
    users.groups.nixling-otel-relay = { };

    users.users.nixling-otel-relay = lockedSystemUser
      "nixling observability vsock relay"
      "nixling-otel-relay";

    systemd.services."nixling-otel-relay@".serviceConfig = {
      User = lib.mkForce "nixling-otel-relay";
      Group = lib.mkForce "nixling-otel-relay";
      DynamicUser = lib.mkForce false;
      SupplementaryGroups = lib.mkForce [ ];
      ExecStartPre = lib.mkBefore [ "+${otelAclRefreshBin}" ];
    };
  })

  (lib.mkIf obsVmEnabled {
    users.groups.nixling-otel-bridge = { };

    users.users.nixling-otel-bridge = lockedSystemUser
      "nixling observability host bridge"
      "nixling-otel-bridge";

    systemd.services.nixling-otel-host-bridge.serviceConfig.ExecStartPre =
      lib.mkBefore [ "+${otelAclRefreshBin}" ];
  })

  (lib.mkIf (obsVmEnabled || relayVmNames != [ ] || chExporterEnabled) {
    environment.systemPackages = [ otelAclRefresh ];

    system.activationScripts.nixlingOtelSocketAcls =
      lib.stringAfter [ "users" ] otelAclRefreshBin;
  })
]
