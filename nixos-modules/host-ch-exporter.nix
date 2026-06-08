# Host-side Cloud Hypervisor metrics exporter.
#
# Wave-1 todo: `component-ch-exporter`.
# Installs the `nixling-ch-exporter.service`, its dedicated system user,
# and the ACL wiring that lets the exporter read per-VM CH API sockets.
{ config, lib, pkgs, ... }:

let
  cfg = config.nixling.observability;

  manifestPath = "/run/current-system/sw/share/nixling/vms.json";
  metricsPath = "/run/nixling-ch-exporter/metrics.prom";
  otelAclRefreshCommand = "/run/current-system/sw/bin/nixling-otel-acl-refresh";

  nixlingChExporter = pkgs.writeShellApplication {
    name = "nixling-ch-exporter";
    runtimeInputs = with pkgs; [
      coreutils
      curl
      jq
      gnused
      socat
      systemd
    ];
    # SC2034: the script unpacks an HTTP request-line with
    #   read -r method path http_version <<< "$request_line"
    # but only consumes `path` — `method` and `http_version` are
    # positional placeholders. Disable the warning rather than
    # awkwardly working around bash's `read` API.
    excludeShellChecks = [ "SC2034" ];
    text = ''
      set -euo pipefail

      MANIFEST=${lib.escapeShellArg manifestPath}
      METRICS_FILE=${lib.escapeShellArg metricsPath}
      SCRAPE_INTERVAL=10
      PORT=9101
      INCLUDE_TOPOLOGY_LABELS=${lib.escapeShellArg (if cfg.ch.exporter.includeTopologyLabels then "1" else "0")}

      declare -A SCRAPE_ERRORS=()
      declare -A UNKNOWN_COUNTERS=()
      KNOWN_STATES=(Created Running Shutdown Paused)

      vm_running() {
        local vm="$1"
        # Headless VMs run via microvm@<vm>.service. Graphics VMs run
        # via nixling-<vm>-gpu.service (the GPU sidecar IS the CH runner
        # for graphics VMs — they bypass microvm@ entirely). Either is
        # enough to consider the VM running. (panel-w3r3 software-1 /
        # nixos-1 / networking-1 / observability-1)
        systemctl is-active --quiet "microvm@$vm.service" 2>/dev/null \
          || systemctl is-active --quiet "nixling-$vm-gpu.service" 2>/dev/null
      }

      usage() {
        cat <<EOF
      usage: nixling-ch-exporter [--port PORT]
             nixling-ch-exporter --serve-once <metrics-file>
      EOF
      }

      prom_escape_label() {
        local value="$1"
        value=''${value//\\/\\\\}
        value=''${value//\"/\\\"}
        value=''${value//$'\n'/\\n}
        printf '%s' "$value"
      }

      sanitize_metric_component() {
        printf '%s' "$1" \
          | tr '[:upper:]' '[:lower:]' \
          | sed -E 's/[^a-z0-9]+/_/g; s/^_+//; s/_+$//; s/_+/_/g'
      }

      vm_env_labels() {
        local vm="$1"
        local env="$2"

        printf 'vm="%s",env="%s"' \
          "$(prom_escape_label "$vm")" \
          "$(prom_escape_label "$env")"
      }

      classify_counter_device() {
        local device="$1"

        case "$device" in
          _net|_net[0-9]*) printf 'virtio_net' ;;
          _disk|_disk[0-9]*) printf 'virtio_blk' ;;
          _fs|_fs[0-9]*) printf 'virtio_fs' ;;
          _pmem|_pmem[0-9]*) printf 'virtio_pmem' ;;
          __rng) printf 'virtio_rng' ;;
          __balloon) printf 'virtio_balloon' ;;
          __console) printf 'virtio_console' ;;
          *) return 1 ;;
        esac
      }

      counter_name_allowed() {
        local device_class="$1"
        local name="$2"

        case "$device_class" in
          virtio_net)
            case "$name" in
              tx_bytes|rx_bytes|tx_frames|rx_frames) return 0 ;;
            esac
            ;;
          virtio_blk)
            case "$name" in
              read_bytes|write_bytes|read_ops|write_ops|read_latency_min|read_latency_max|read_latency_avg|write_latency_min|write_latency_max|write_latency_avg) return 0 ;;
            esac
            ;;
          virtio_fs|virtio_pmem|virtio_rng|virtio_balloon|virtio_console)
            return 1
            ;;
        esac

        return 1
      }

      common_labels() {
        local vm="$1"
        local env="$2"
        local role="$3"
        local graphics="$4"
        local tpm="$5"
        local audio="$6"
        local usbip_yubikey="$7"
        local bridge="$8"
        local tap="$9"

        if [ "$INCLUDE_TOPOLOGY_LABELS" = "1" ]; then
          printf 'vm="%s",env="%s",role="%s",graphics="%s",tpm="%s",audio="%s",usbip_yubikey="%s",bridge="%s",tap="%s"' \
            "$(prom_escape_label "$vm")" \
            "$(prom_escape_label "$env")" \
            "$(prom_escape_label "$role")" \
            "$(prom_escape_label "$graphics")" \
            "$(prom_escape_label "$tpm")" \
            "$(prom_escape_label "$audio")" \
            "$(prom_escape_label "$usbip_yubikey")" \
            "$(prom_escape_label "$bridge")" \
            "$(prom_escape_label "$tap")"
        else
          printf 'vm="%s",env="%s",role="%s"' \
            "$(prom_escape_label "$vm")" \
            "$(prom_escape_label "$env")" \
            "$(prom_escape_label "$role")"
        fi
      }

      curl_json() {
        local socket="$1"
        local path="$2"

        curl \
          --silent \
          --show-error \
          --fail \
          --max-time 5 \
          --unix-socket "$socket" \
          --header 'Accept: application/json' \
          "http://localhost''${path}"
      }

      manifest_rows() {
        jq -r '
          . as $root
          | keys[]
          | select(startswith("_") | not)
          | . as $name
          | $root[$name] as $vm
          | [
              $name,
              ($vm.apiSocket // ""),
              ($vm.env // ""),
              (if $name == ($root._observability.vmName // "") then "obs"
               elif ($vm.isNetVm // false) then "router"
               else "workload"
               end),
              (($vm.graphics // false) | tostring),
              (($vm.tpm // false) | tostring),
              (($vm.audio // false) | tostring),
              (($vm.usbipYubikey // false) | tostring),
              ($vm.bridge // ""),
              ($vm.tap // ""),
              (if ($vm.observability.enabled // false) then 1 else 0 end)
            ]
          | @tsv
        ' "$MANIFEST"
      }

      contains_state() {
        local needle="$1"
        shift

        local candidate
        for candidate in "$@"; do
          if [ "$candidate" = "$needle" ]; then
            return 0
          fi
        done
        return 1
      }

      serve_once() {
        local file="$1"
        local request_line=""
        local method=""
        local path="/"
        local http_version=""
        local line=""

        if IFS= read -r request_line; then
          request_line=''${request_line%$'\r'}
          IFS=' ' read -r method path http_version <<< "$request_line"
        fi
        while IFS= read -r line; do
          line=''${line%$'\r'}
          [ -z "$line" ] && break
        done

        if [ "$path" = "/metrics" ]; then
          printf 'HTTP/1.1 200 OK\r\n'
          printf 'Content-Type: text/plain; version=0.0.4; charset=utf-8\r\n'
          printf 'Connection: close\r\n'
          printf '\r\n'
          if [ -f "$file" ]; then
            cat "$file"
          else
            printf '# exporter warming up\n'
          fi
          return 0
        fi

        printf 'HTTP/1.1 404 Not Found\r\n'
        printf 'Content-Type: text/plain; charset=utf-8\r\n'
        printf 'Connection: close\r\n'
        printf '\r\n'
        printf 'not found\n'
      }

      refresh_metrics_once() {
        local rows=""
        local tmp_file="''${METRICS_FILE}.new"

        if [ ! -r "$MANIFEST" ]; then
          echo "nixling-ch-exporter: manifest $MANIFEST is unreadable" >&2
          return 1
        fi

        if ! rows="$(manifest_rows)"; then
          echo "nixling-ch-exporter: failed to parse $MANIFEST" >&2
          return 1
        fi

        {
          echo '# HELP nixling_vm_ch_api_up Whether the VM Cloud Hypervisor API responded to /vmm.ping.'
          echo '# TYPE nixling_vm_ch_api_up gauge'
          echo '# HELP nixling_vm_state Cloud Hypervisor VM state exported as a one-hot gauge per state label.'
          echo '# TYPE nixling_vm_state gauge'
          echo '# HELP nixling_vm_running Whether the host currently considers the VM running.'
          echo '# TYPE nixling_vm_running gauge'
          echo '# HELP nixling_vm_observability_enabled Whether the VM has observability enabled in the manifest.'
          echo '# TYPE nixling_vm_observability_enabled gauge'
          echo '# HELP nixling_vm_last_scrape_timestamp_seconds Unix timestamp of the last scrape attempt for this VM.'
          echo '# TYPE nixling_vm_last_scrape_timestamp_seconds gauge'
          echo '# HELP nixling_vm_scrape_errors_total Total number of failed Cloud Hypervisor scrape cycles for this VM.'
          echo '# TYPE nixling_vm_scrape_errors_total counter'
          echo '# HELP nixling_vm_unknown_counters_total Total number of dropped Cloud Hypervisor counters with unknown device/name pairs.'
          echo '# TYPE nixling_vm_unknown_counters_total counter'

          declare -A emitted_counter_headers=()

          while IFS=$'\t' read -r vm socket env role graphics tpm audio usbip_yubikey bridge tap observability_enabled; do
            [ -n "$vm" ] || continue

            local_labels="$(common_labels "$vm" "$env" "$role" "$graphics" "$tpm" "$audio" "$usbip_yubikey" "$bridge" "$tap")"
            unknown_counter_labels="$(vm_env_labels "$vm" "$env")"
            now_epoch="$(date +%s)"
            running=0
            api_up=0
            state=""
            scrape_failed=0
            counters_rows=""
            unknown_counter_hits=0

            if vm_running "$vm"; then
              running=1
            fi

            if [ -S "$socket" ]; then
              if curl_json "$socket" '/api/v1/vmm.ping' >/dev/null; then
                api_up=1
              else
                scrape_failed=1
              fi

              if [ "$api_up" -eq 1 ]; then
                if info_json="$(curl_json "$socket" '/api/v1/vm.info')"; then
                  if ! state="$(printf '%s' "$info_json" | jq -r '.state // empty')"; then
                    state=""
                    scrape_failed=1
                  fi
                else
                  scrape_failed=1
                fi
              fi

              if [ "$api_up" -eq 1 ] && [ "$scrape_failed" -eq 0 ]; then
                case "$state" in
                  Running|Paused)
                    if counters_json="$(curl_json "$socket" '/api/v1/vm.counters')"; then
                      if ! counters_rows="$(printf '%s' "$counters_json" | jq -r '
                        to_entries[]?
                        | .key as $device
                        | (.value // {})
                        | to_entries[]?
                        | [ $device, .key, (.value | tostring) ]
                        | @tsv
                      ')"; then
                        counters_rows=""
                        scrape_failed=1
                      fi
                    else
                      scrape_failed=1
                    fi
                    ;;
                esac
              fi
            fi

            if [ "$scrape_failed" -eq 1 ]; then
              SCRAPE_ERRORS["$vm"]=$(( ''${SCRAPE_ERRORS["$vm"]:-0} + 1 ))
            fi

            printf 'nixling_vm_ch_api_up{%s} %s\n' "$local_labels" "$api_up"

            states=("''${KNOWN_STATES[@]}")
            if [ -n "$state" ] && ! contains_state "$state" "''${states[@]}"; then
              states+=("$state")
            fi
            for candidate_state in "''${states[@]}"; do
              state_value=0
              if [ "$candidate_state" = "$state" ]; then
                state_value=1
              fi
              printf 'nixling_vm_state{%s,state="%s"} %s\n' \
                "$local_labels" \
                "$(prom_escape_label "$candidate_state")" \
                "$state_value"
            done

            printf 'nixling_vm_running{%s} %s\n' "$local_labels" "$running"
            printf 'nixling_vm_observability_enabled{%s} %s\n' \
              "$local_labels" \
              "$observability_enabled"

            if [ -n "$counters_rows" ]; then
              while IFS=$'\t' read -r device name value; do
                [ -n "$device" ] || continue

                if ! device_class="$(classify_counter_device "$device")"; then
                  unknown_counter_hits=$((unknown_counter_hits + 1))
                  continue
                fi
                if ! counter_name_allowed "$device_class" "$name"; then
                  unknown_counter_hits=$((unknown_counter_hits + 1))
                  continue
                fi

                metric_device="$(sanitize_metric_component "$device_class")"
                metric_name="$(sanitize_metric_component "$name")"
                metric_family="nixling_vm_counter_''${metric_device}_''${metric_name}"
                if [ -z "''${emitted_counter_headers["$metric_family"]:-}" ]; then
                  printf '# HELP %s Cloud Hypervisor counter %s.%s.\n' \
                    "$metric_family" \
                    "$device_class" \
                    "$name"
                  printf '# TYPE %s gauge\n' "$metric_family"
                  emitted_counter_headers["$metric_family"]=1
                fi
                printf '%s{%s,device="%s",name="%s"} %s\n' \
                  "$metric_family" \
                  "$local_labels" \
                  "$(prom_escape_label "$device")" \
                  "$(prom_escape_label "$name")" \
                  "$value"
              done <<< "$counters_rows"
            fi

            if [ "$unknown_counter_hits" -gt 0 ]; then
              UNKNOWN_COUNTERS["$vm"]=$(( ''${UNKNOWN_COUNTERS["$vm"]:-0} + unknown_counter_hits ))
            fi
            printf 'nixling_vm_unknown_counters_total{%s} %s\n' \
              "$unknown_counter_labels" \
              "''${UNKNOWN_COUNTERS["$vm"]:-0}"

            printf 'nixling_vm_last_scrape_timestamp_seconds{%s} %s\n' \
              "$local_labels" \
              "$now_epoch"
            printf 'nixling_vm_scrape_errors_total{%s} %s\n' \
              "$local_labels" \
              "''${SCRAPE_ERRORS["$vm"]:-0}"
          done <<< "$rows"
        } > "$tmp_file"

        mv -f "$tmp_file" "$METRICS_FILE"
      }

      poller_loop() {
        while true; do
          refresh_metrics_once
          sleep "$SCRAPE_INTERVAL"
        done
      }

      listener_loop() {
        local system_cmd=""
        printf -v system_cmd '%q --serve-once %q' "$0" "$METRICS_FILE"
        exec socat -T 30 \
          "TCP-LISTEN:''${PORT},fork,reuseaddr,bind=127.0.0.1" \
          "SYSTEM:''${system_cmd}"
      }

      if [ "''${1:-}" = "--serve-once" ]; then
        if [ "$#" -ne 2 ]; then
          usage >&2
          exit 2
        fi
        serve_once "$2"
        exit 0
      fi

      while [ "$#" -gt 0 ]; do
        case "$1" in
          --port)
            [ "$#" -ge 2 ] || {
              usage >&2
              exit 2
            }
            PORT="$2"
            shift 2
            ;;
          -h|--help)
            usage
            exit 0
            ;;
          *)
            usage >&2
            exit 2
            ;;
        esac
      done

      install -d -m 0700 "$(dirname "$METRICS_FILE")"
      refresh_metrics_once

      poller_loop &
      poller_pid=$!
      listener_loop &
      listener_pid=$!

      cleanup() {
        kill "$poller_pid" "$listener_pid" 2>/dev/null || true
        wait "$poller_pid" "$listener_pid" 2>/dev/null || true
      }
      trap cleanup EXIT INT TERM

      wait -n "$poller_pid" "$listener_pid"
      status=$?
      cleanup
      exit "$status"
    '';
  };
in
lib.mkIf (cfg.enable && cfg.ch.exporter.enable) {
  users.groups.nixling-ch-exporter = { };

  users.users.nixling-ch-exporter = {
    isSystemUser = true;
    group = "nixling-ch-exporter";
    description = "nixling Cloud Hypervisor metrics exporter";
    home = "/var/empty";
    createHome = false;
    hashedPassword = "!";
    shell = pkgs.shadow + "/bin/nologin";
  };

  systemd.services."microvm@".serviceConfig.ExecStartPost = lib.mkAfter [
    "+${otelAclRefreshCommand}"
  ];

  systemd.services.nixling-ch-exporter = {
    description = "nixling Cloud Hypervisor metrics exporter";
    wantedBy = [ "multi-user.target" ];
    restartIfChanged = false;
    startLimitBurst = 20;
    startLimitIntervalSec = 300;

    serviceConfig = {
      Type = "exec";
      User = "nixling-ch-exporter";
      Group = "nixling-ch-exporter";
      DynamicUser = false;
      RuntimeDirectory = "nixling-ch-exporter";
      RuntimeDirectoryMode = "0700";
      UMask = "0077";
      ExecStartPre = lib.mkBefore [ "+${otelAclRefreshCommand}" ];
      ExecStart = "${nixlingChExporter}/bin/nixling-ch-exporter --port ${toString cfg.ch.exporter.listenPort}";
      Restart = "on-failure";
      RestartSec = "5s";
      NoNewPrivileges = true;
      ProtectSystem = "strict";
      ProtectHome = true;
      PrivateTmp = true;
      PrivateDevices = true;
      RestrictAddressFamilies = [ "AF_UNIX" "AF_INET" ];
      SystemCallFilter = [ "@system-service" "~@privileged" "~@resources" ];
      CapabilityBoundingSet = "";
      AmbientCapabilities = "";
      ReadWritePaths = [ ];
      ReadOnlyPaths = [
        "/run/current-system/sw/share/nixling"
        "/var/lib/nixling"
      ];
    };
  };
}
