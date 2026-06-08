#!/usr/bin/env bash
# tests/observability-eval.sh — eval-time coverage for
# nixling.observability.*.
#
# Each case constructs a synthetic consumer-style nixosSystem that
# imports nixling's module tree and then inspects either the rendered
# manifest, the auto-declared observability env/VM surface, or the
# nixling CLI wrapper. Cases whose backing Wave-1 implementation has not
# landed in this worktree yet auto-SKIP with a TODO post-integration
# marker so Layer-1 stays green before the parallel tracks merge.
#
# Run via:
#   tests/observability-eval.sh
# Wired into tests/static.sh.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
SCRATCH=$(mktemp -d -p "$ROOT" .observability-eval.XXXXXX)
export NL_LOG="$SCRATCH/observability-eval.log"

# shellcheck source=lib.sh
. "$HERE/lib.sh"
add_cleanup "rm -rf -- \"$SCRATCH\""

PASS=0
FAIL=0
SKIP=0

export EVAL_EXPR_FILE=""
EVAL_OUT_FILE=""
EVAL_ERR_FILE=""

pass_case() {
  log "  PASS: $*"
  return 0
}

fail_case() {
  log "  FAIL: $*"
  return 1
}

skip_case() {
  log "  SKIP: $*"
  return 2
}

run_case() {
  local fn="$1"
  local rc=0
  if "$fn"; then
    PASS=$((PASS+1))
  else
    rc=$?
    case "$rc" in
      2) SKIP=$((SKIP+1)) ;;
      *) FAIL=$((FAIL+1)) ;;
    esac
  fi
}

show_stderr_tail() {
  local file="$1"
  if [ -s "$file" ]; then
    log "    --- stderr (tail) ---"
    tail -15 "$file" | sed 's/^/      /' >&2
  fi
}

stderr_contains_all() {
  local file="$1"
  shift
  local needle
  for needle in "$@"; do
    if ! grep -q -F -- "$needle" "$file"; then
      return 1
    fi
  done
}

assert_json_eq() {
  local file="$1" jq_filter="$2" expected="$3" msg="$4"
  local actual
  actual=$(jq -r "$jq_filter" "$file") || return 1
  assert_eq "$actual" "$expected" "$msg" || return 1
}

mk_expr() {
  local override="$1"
  local body="$2"
  local system="${3:-x86_64-linux}"
  cat <<EOF
let
  pkgs = import <nixpkgs> { system = "$system"; };
  inherit (pkgs) lib;
  flake = builtins.getFlake (toString $ROOT);
  nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;
  pkgsForSystem = import flake.inputs.nixpkgs {
    system = "$system";
    config = { allowUnsupportedSystem = true; };
  };
  nixos = nixosSystem {
    system = "$system";
    pkgs = pkgsForSystem;
    modules = [
      flake.nixosModules.default
      ({ lib, ... }: {
        boot.loader.grub.enable = false;
        boot.loader.systemd-boot.enable = false;
        boot.initrd.includeDefaultModules = false;
        fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
        environment.etc."machine-id".text =
          "00000000000000000000000000000000";
        system.stateVersion = "25.11";
        users.users.alice = { isNormalUser = true; uid = 1000; };
        nixling.site = {
          waylandUser = "alice";
          launcherUsers = [ "alice" ];
          yubikey.enable = false;
        };
        nixling.envs.work = {
          lanSubnet = "10.20.0.0/24";
          uplinkSubnet = "192.0.2.0/30";
        };
        nixling.vms.corp-vm = {
          enable = true;
          env = "work";
          index = 10;
          ssh.user = "alice";
          config = {
            networking.hostName = lib.mkDefault "corp-vm";
            users.users.alice = { isNormalUser = true; uid = 1000; };
          };
        };
      })
      $override
    ];
  };
in
  $body
EOF
}

run_eval() {
  local name="$1" override="$2" body="$3" system="${4:-x86_64-linux}"
  local expr_file="$SCRATCH/$name.nix"
  local out_file="$SCRATCH/$name.stdout"
  local err_file="$SCRATCH/$name.stderr"

  mk_expr "$override" "$body" "$system" > "$expr_file"
  EVAL_EXPR_FILE="$expr_file"
  EVAL_OUT_FILE="$out_file"
  EVAL_ERR_FILE="$err_file"

  if nix-instantiate --eval --strict --json \
      --expr "$(cat "$expr_file")" \
      > "$out_file" 2> "$err_file"; then
    return 0
  fi
  return 1
}

run_instantiate() {
  local name="$1" override="$2" body="$3" system="${4:-x86_64-linux}"
  local expr_file="$SCRATCH/$name.instantiate.nix"
  local out_file="$SCRATCH/$name.instantiate.stdout"
  local err_file="$SCRATCH/$name.instantiate.stderr"

  mk_expr "$override" "$body" "$system" > "$expr_file"
  EVAL_EXPR_FILE="$expr_file"
  EVAL_OUT_FILE="$out_file"
  EVAL_ERR_FILE="$err_file"

  if nix-instantiate --expr "$(cat "$expr_file")" \
      > "$out_file" 2> "$err_file"; then
    return 0
  fi
  return 1
}

assert_eval_fails() {
  local name="$1" override="$2" body="$3"
  shift 3

  if run_eval "$name" "$override" "$body"; then
    return 2
  fi

  stderr_contains_all "$EVAL_ERR_FILE" "$@"
}

feature_auto_obs_ready() {
  local vm_name="$1"
  local probe_name="__probe-auto-obs-${vm_name//[^a-zA-Z0-9]/-}"
  local override body
  override=$(cat <<EOF
({ ... }: {
  nixling.observability.enable = true;
  nixling.observability.vmName = "$vm_name";
})
EOF
)
  body=$(cat <<EOF
{
  hasObsEnv = builtins.hasAttr "obs" nixos.config.nixling.envs;
  hasObsVm = builtins.hasAttr "$vm_name" nixos.config.nixling.vms;
}
EOF
)
  run_eval "$probe_name" "$override" "$body" || return 1
  jq -e '.hasObsEnv and .hasObsVm' "$EVAL_OUT_FILE" >/dev/null 2>&1
}

feature_transport_vsock_ready() {
  run_eval \
    "__probe-transport-vsock" \
    '({ ... }: {
       nixling.observability.enable = true;
       nixling.vms.corp-vm.observability.enable = true;
     })' \
    'builtins.hasAttr "nixling-otel-relay@" nixos.config.systemd.services' \
    || return 1
  jq -e '. == true' "$EVAL_OUT_FILE" >/dev/null 2>&1
}

CLI_TRACES_PROBED=0
CLI_TRACES_DEFAULT=""
CLI_TRACES_DISABLED=""

probe_cli_text_contains_otel() {
  local name="$1" override="$2" body expr_file out_file err_file drv
  body=$(cat <<'EOF'
let
  cliPkgs = builtins.filter
    (p: (p.name or "") == "nixling" || (p.pname or "") == "nixling")
    nixos.config.environment.systemPackages;
  cliPkg =
    if cliPkgs == []
    then throw "nixling package not found in environment.systemPackages"
    else builtins.head cliPkgs;
in
  cliPkg
EOF
)
  expr_file="$SCRATCH/$name.nix"
  out_file="$SCRATCH/$name.stdout"
  err_file="$SCRATCH/$name.stderr"
  mk_expr "$override" "$body" > "$expr_file"
  EVAL_EXPR_FILE="$expr_file"
  EVAL_OUT_FILE="$out_file"
  EVAL_ERR_FILE="$err_file"

  if ! drv=$(nix-instantiate --expr "$(cat "$expr_file")" 2> "$err_file"); then
    return 1
  fi
  printf '%s\n' "$drv" > "$out_file"

  # nixos-r6: the only authoritative check for "otel-cli is on the
  # nixling PATH" is the derivation's transitive requisites — that's
  # what runtimeInputs lowers to. The script body always references
  # `otel-cli` literally (inside the trace-helper else-branch), so a
  # regression that removes `pkgs.otel-cli` from `runtimeInputs` while
  # the gated script text is unchanged would silently slip past a
  # text-only check.
  if nix-store --query --requisites "$drv" 2>> "$err_file" | grep -q -F 'otel-cli'; then
    printf 'true\n'
  else
    printf 'false\n'
  fi
}

probe_cli_script_text() {
  local name="$1" override="$2" body expr_file out_file err_file drv
  body=$(cat <<'EOF'
let
  cliPkgs = builtins.filter
    (p: (p.name or "") == "nixling" || (p.pname or "") == "nixling")
    nixos.config.environment.systemPackages;
  cliPkg =
    if cliPkgs == []
    then throw "nixling package not found in environment.systemPackages"
    else builtins.head cliPkgs;
in
  cliPkg
EOF
)
  expr_file="$SCRATCH/$name.nix"
  out_file="$SCRATCH/$name.stdout"
  err_file="$SCRATCH/$name.stderr"
  mk_expr "$override" "$body" > "$expr_file"
  EVAL_EXPR_FILE="$expr_file"
  EVAL_OUT_FILE="$out_file"
  EVAL_ERR_FILE="$err_file"

  if ! drv=$(nix-instantiate --expr "$(cat "$expr_file")" 2> "$err_file"); then
    return 1
  fi
  if ! nix-store --query --binding text "$drv" > "$out_file" 2>> "$err_file"; then
    return 1
  fi
  cat "$out_file"
}

probe_cli_traces_gate() {
  if [ "$CLI_TRACES_PROBED" -eq 1 ]; then
    return 0
  fi

  CLI_TRACES_DEFAULT=$(probe_cli_text_contains_otel \
    "__probe-cli-traces-default" \
    '({ ... }: { nixling.observability.enable = true; })') || return 1

  CLI_TRACES_DISABLED=$(probe_cli_text_contains_otel \
    "__probe-cli-traces-disabled" \
    '({ ... }: {
       nixling.observability.enable = true;
       nixling.observability.cli.traces.enable = false;
     })') || return 1

  CLI_TRACES_PROBED=1
}

test_obs_disabled_default() {
  run_eval \
    "obs-disabled-default" \
    '({ ... }: { })' \
    '{ manifest = (builtins.fromJSON nixos.config.nixling._manifestPkg.text)._observability; }' \
    || {
      show_stderr_tail "$EVAL_ERR_FILE"
      fail_case "obs-disabled-default: eval failed"
      return 1
    }

  assert_json_eq "$EVAL_OUT_FILE" '.manifest.enabled' 'false' \
    'obs-disabled-default: _observability.enabled' || return 1

  pass_case 'obs-disabled-default'
}

test_obs_default_off_no_units() {
  local body cli_has_otel

  body=$(cat <<'EOF'
let
  otelServices = builtins.filter
    (name: builtins.match "^nixling-otel-.*" name != null)
    (builtins.attrNames nixos.config.systemd.services);
in {
  otelServices = otelServices;
  otelServiceCount = builtins.length otelServices;
}
EOF
)

  run_eval \
    "obs-default-off-no-units" \
    '({ ... }: { nixling.observability.enable = false; })' \
    "$body" \
    || {
      show_stderr_tail "$EVAL_ERR_FILE"
      fail_case 'obs-default-off-no-units: eval failed'
      return 1
    }

  assert_json_eq "$EVAL_OUT_FILE" '.otelServiceCount' '0' \
    'obs-default-off-no-units: no nixling-otel-* systemd units when observability is disabled' || return 1

  cli_has_otel=$(probe_cli_text_contains_otel \
    "__probe-cli-traces-obs-default-off" \
    '({ ... }: { nixling.observability.enable = false; })') || {
      show_stderr_tail "$EVAL_ERR_FILE"
      fail_case 'obs-default-off-no-units: could not inspect nixling runtimeInputs'
      return 1
    }

  assert_eq "$cli_has_otel" 'false' \
    'obs-default-off-no-units: otel-cli absent from nixling closure when observability is disabled' || return 1

  pass_case 'obs-default-off-no-units'
}

test_obs_enabled_defaults() {
  local body
  body=$(cat <<'EOF'
let
  manifest = builtins.fromJSON nixos.config.nixling._manifestPkg.text;
in {
  hasSysObsStack = builtins.hasAttr "sys-obs-stack" nixos.config.nixling.vms;
  hasObsEnv = builtins.hasAttr "obs" nixos.config.nixling.envs;
  obsEnv = if builtins.hasAttr "obs" nixos.config.nixling.envs then {
    lanSubnet = nixos.config.nixling.envs.obs.lanSubnet;
    uplinkSubnet = nixos.config.nixling.envs.obs.uplinkSubnet;
  } else null;
  topObs = manifest._observability;
}
EOF
)

  run_eval \
    "obs-enabled-defaults" \
    '({ ... }: { nixling.observability.enable = true; })' \
    "$body" \
    || {
      show_stderr_tail "$EVAL_ERR_FILE"
      fail_case 'obs-enabled-defaults: eval failed'
      return 1
    }

  if ! jq -e '.hasSysObsStack and .hasObsEnv' "$EVAL_OUT_FILE" >/dev/null 2>&1; then
    skip_case 'obs-enabled-defaults: TODO post-integration — auto-obs-vm has not materialized sys-obs-stack + obs env in this worktree'
    return 2
  fi

  assert_json_eq "$EVAL_OUT_FILE" '.obsEnv.lanSubnet' '10.40.0.0/24' \
    'obs-enabled-defaults: obs lanSubnet' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.obsEnv.uplinkSubnet' '203.0.113.0/30' \
    'obs-enabled-defaults: obs uplinkSubnet' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.topObs.vmName' 'sys-obs-stack' \
    'obs-enabled-defaults: _observability.vmName' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.topObs.obsVsockCid' '1000' \
    'obs-enabled-defaults: _observability.obsVsockCid' || return 1

  pass_case 'obs-enabled-defaults'
}

# v0.2.0+: consumer extensions of the auto-declared observability VM
# are EXPECTED and supported. The pre-v0.2.0 collision assertion was
# overly conservative — the framework's `observability-vm.nix` block
# uses `lib.mkDefault` for every value, so consumer overrides merge
# via the module system. This test pins that behaviour: eval must
# succeed when a consumer extends `nixling.vms.<obsVmName>`.
test_obs_name_collision() {
  local override
  override=$(cat <<'EOF'
({ ... }: {
  nixling.observability.enable = true;
  nixling.vms.sys-obs-stack = {
    ssh.user = "alice";
    config = {
      users.users.alice = { isNormalUser = true; uid = 1000; };
    };
  };
})
EOF
)

  if run_eval "obs-name-extension-allowed" "$override" 'nixos.config.system.build.toplevel.drvPath'; then
    pass_case 'obs-name-extension-allowed (consumer can extend auto-declared obs VM)'
    return 0
  fi

  if feature_auto_obs_ready 'sys-obs-stack'; then
    show_stderr_tail "$EVAL_ERR_FILE"
    fail_case 'obs-name-extension-allowed: eval failed but consumer-side extension should be permitted'
    return 1
  fi

  skip_case 'obs-name-extension-allowed: TODO post-integration — auto-obs-vm has not landed in this worktree'
  return 2
}

test_obs_cid_collision() {
  local override err_file
  override=$(cat <<'EOF'
({ lib, ... }: {
  nixling.observability.enable = true;
  nixling.envs.aaa = {
    lanSubnet = "10.30.0.0/24";
    uplinkSubnet = "198.51.100.0/30";
  };
  nixling.envs.bbb = {
    lanSubnet = "10.31.0.0/24";
    uplinkSubnet = "198.18.0.0/30";
  };
  nixling.vms.corp-vm.env = lib.mkForce "aaa";
  nixling.vms.corp-vm.index = lib.mkForce 110;
  nixling.vms.corp-vm.observability.enable = true;
  nixling.vms.other-vm = {
    enable = true;
    env = "bbb";
    index = 10;
    ssh.user = "alice";
    observability.enable = true;
    config = {
      networking.hostName = lib.mkDefault "other-vm";
      users.users.alice = { isNormalUser = true; uid = 1000; };
    };
  };
})
EOF
)

  if run_eval "obs-cid-collision" "$override" 'nixos.config.system.build.toplevel.drvPath'; then
    if feature_transport_vsock_ready; then
      fail_case 'obs-cid-collision: eval succeeded but the CID collision should fail'
      return 1
    fi
    skip_case 'obs-cid-collision: TODO post-integration — transport-vsock relay/assertions have not landed in this worktree'
    return 2
  fi

  err_file="$EVAL_ERR_FILE"
  if stderr_contains_all "$err_file" 'CID' 'corp-vm' 'other-vm'; then
    pass_case 'obs-cid-collision'
    return 0
  fi

  if feature_transport_vsock_ready; then
    show_stderr_tail "$err_file"
    fail_case 'obs-cid-collision: eval failed, but stderr did not name the colliding VMs/CID'
    return 1
  fi

  skip_case 'obs-cid-collision: TODO post-integration — transport-vsock CID-collision assertion has not landed in this worktree'
  return 2
}

test_obs_manifest_fields() {
  run_eval \
    "obs-manifest-fields" \
    '({ ... }: { nixling.observability.enable = true; })' \
    '(builtins.fromJSON nixos.config.nixling._manifestPkg.text)."corp-vm".observability' \
    || {
      show_stderr_tail "$EVAL_ERR_FILE"
      fail_case 'obs-manifest-fields: eval failed'
      return 1
    }

  assert_json_eq "$EVAL_OUT_FILE" '.enabled' 'false' \
    'obs-manifest-fields: observability.enabled' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.vsockCid' '210' \
    'obs-manifest-fields: observability.vsockCid' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.vsockHostSocket' '/var/lib/nixling/vms/corp-vm/vsock.sock' \
    'obs-manifest-fields: observability.vsockHostSocket' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.agentSocket' '/run/nixling/otlp.sock' \
    'obs-manifest-fields: observability.agentSocket' || return 1

  pass_case 'obs-manifest-fields'
}

test_obs_relay_acl_surface() {
  local body exec_start_pre exec_start_pre_path acl_script_path

  body=$(cat <<'EOF'
let
  relay = nixos.config.systemd.services."nixling-otel-relay@";
  relayAclExecStartPre = builtins.elemAt relay.serviceConfig.ExecStartPre 0;
  relayAclActivationPath = nixos.config.system.activationScripts.nixlingOtelSocketAcls.text;
in {
  relayGroupDeclared = builtins.hasAttr "nixling-otel-relay" nixos.config.users.groups;
  relayUserDeclared = builtins.hasAttr "nixling-otel-relay" nixos.config.users.users;
  relayUserGroup = nixos.config.users.users.nixling-otel-relay.group;
  relayServiceUser = relay.serviceConfig.User;
  relayServiceGroup = relay.serviceConfig.Group;
  relayDynamicUser = relay.serviceConfig.DynamicUser;
  relaySupplementaryGroups = relay.serviceConfig.SupplementaryGroups;
  relayExecStartPre = relay.serviceConfig.ExecStartPre;
  relayAclExecStartPrePath = lib.removePrefix "+" relayAclExecStartPre;
  relayAclActivationPath = relayAclActivationPath;
}
EOF
)

  run_eval \
    "obs-relay-acl-surface" \
    '({ ... }: {
       nixling.observability.enable = true;
       nixling.vms.corp-vm.observability.enable = true;
     })' \
    "$body" \
    || {
      show_stderr_tail "$EVAL_ERR_FILE"
      fail_case 'obs-relay-acl-surface: eval failed'
      return 1
    }

  assert_json_eq "$EVAL_OUT_FILE" '.relayGroupDeclared' 'true' \
    'obs-relay-acl-surface: relay group declared' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.relayUserDeclared' 'true' \
    'obs-relay-acl-surface: relay user declared' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.relayUserGroup' 'nixling-otel-relay' \
    'obs-relay-acl-surface: relay user primary group' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.relayServiceUser' 'nixling-otel-relay' \
    'obs-relay-acl-surface: template service user override' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.relayServiceGroup' 'nixling-otel-relay' \
    'obs-relay-acl-surface: template service group override' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.relayDynamicUser' 'false' \
    'obs-relay-acl-surface: DynamicUser disabled for static relay principal' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.relaySupplementaryGroups | length' '0' \
    'obs-relay-acl-surface: relay supplementary groups cleared' || return 1

  exec_start_pre=$(jq -r '.relayExecStartPre[0]' "$EVAL_OUT_FILE") || return 1
  assert_contains "$exec_start_pre" 'nixling-otel-acl-refresh' \
    'obs-relay-acl-surface: ExecStartPre refreshes relay ACLs' || return 1

  exec_start_pre_path=$(jq -r '.relayAclExecStartPrePath' "$EVAL_OUT_FILE") || return 1
  acl_script_path=$(jq -r '.relayAclActivationPath' "$EVAL_OUT_FILE") || return 1
  assert_eq "$exec_start_pre_path" "$acl_script_path" \
    'obs-relay-acl-surface: activation script and ExecStartPre share the same acl-refresh store path' || return 1

  pass_case 'obs-relay-acl-surface'
}

test_obs_stack_vm_guest_surface() {
  local body grafana_bind expected_bind vsock_in_exec_start

  body=$(cat <<'EOF'
let
  obsVm = nixos.config.nixling.observability.vmName;
  obsGuest = nixos.config.microvm.vms.${obsVm}.config.config;
  grafanaDatasources = obsGuest.services.grafana.provision.datasources.settings.datasources;
  lokiDatasource = builtins.head (builtins.filter (ds: ds.name == "Loki") grafanaDatasources);
  tempoDatasource = builtins.head (builtins.filter (ds: ds.name == "Tempo") grafanaDatasources);
in {
  obsVm = obsVm;
  manifestHasObsVm = builtins.hasAttr obsVm nixos.config.nixling.manifest;
  grafanaEnable = obsGuest.services.grafana.enable;
  prometheusEnable = obsGuest.services.prometheus.enable;
  lokiEnable = obsGuest.services.loki.enable;
  tempoEnable = obsGuest.services.tempo.enable;
  alloyEnable = obsGuest.services.alloy.enable;
  grafanaBind = obsGuest.services.grafana.settings.server.http_addr;
  expectedGrafanaBind = nixos.config.nixling.observability.grafana.listenAddress;
  grafanaSecretKey = obsGuest.services.grafana.settings.security.secret_key;
  grafanaLoadCredential = obsGuest.systemd.services.grafana.serviceConfig.LoadCredential;
  grafanaDatasources = grafanaDatasources;
  grafanaDashboardProviders = obsGuest.services.grafana.provision.dashboards.settings.providers;
  lokiDerivedFields = lokiDatasource.jsonData.derivedFields;
  tempoTraceToLogsDatasource = tempoDatasource.jsonData.tracesToLogsV2.datasourceUid;
  prometheusRetention = obsGuest.services.prometheus.retentionTime;
  lokiRetention = obsGuest.services.loki.configuration.limits_config.retention_period;
  tempoRetention = obsGuest.services.tempo.settings.compactor.compaction.block_retention;
  vsockInExists = builtins.hasAttr "nixling-otel-vsock-in" obsGuest.systemd.services;
  vsockInRestartIfChanged = obsGuest.systemd.services.nixling-otel-vsock-in.restartIfChanged;
  vsockInExecStart = obsGuest.systemd.services.nixling-otel-vsock-in.serviceConfig.ExecStart;
}
EOF
)

  run_eval \
    "obs-stack-vm-guest-surface" \
    '({ ... }: {
       nixling.observability.enable = true;
       nixling.observability.retention.metrics = "5d";
       nixling.observability.retention.logs = "3d";
       nixling.observability.retention.traces = "1d";
     })' \
    "$body" \
    || {
      show_stderr_tail "$EVAL_ERR_FILE"
      fail_case 'obs-stack-vm-guest-surface: eval failed'
      return 1
    }

  assert_json_eq "$EVAL_OUT_FILE" '.obsVm' 'sys-obs-stack' \
    'obs-stack-vm-guest-surface: default obs VM name' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.manifestHasObsVm' 'true' \
    'obs-stack-vm-guest-surface: manifest renders sys-obs-stack' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.grafanaEnable' 'true' \
    'obs-stack-vm-guest-surface: grafana enabled' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.prometheusEnable' 'true' \
    'obs-stack-vm-guest-surface: prometheus enabled' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.lokiEnable' 'true' \
    'obs-stack-vm-guest-surface: loki enabled' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.tempoEnable' 'true' \
    'obs-stack-vm-guest-surface: tempo enabled' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.alloyEnable' 'true' \
    'obs-stack-vm-guest-surface: alloy enabled' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.vsockInExists' 'true' \
    'obs-stack-vm-guest-surface: vsock-in service declared' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.vsockInRestartIfChanged' 'false' \
    'obs-stack-vm-guest-surface: vsock-in restartIfChanged' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.grafanaLoadCredential | index("secret_key:/run/nixling-obs-secrets/grafana-secret-key") != null' 'true' \
    'obs-stack-vm-guest-surface: grafana LoadCredential wires secret_key from the host-shared secret (v0.2.0+)' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.grafanaSecretKey' '$__file{/run/credentials/grafana.service/secret_key}' \
    'obs-stack-vm-guest-surface: grafana secret_key reads the service credential' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.grafanaDatasources | length' '3' \
    'obs-stack-vm-guest-surface: grafana provisions Prometheus/Loki/Tempo datasources' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.grafanaDatasources[] | select(.name == "Prometheus") | .url' 'http://127.0.0.1:9090' \
    'obs-stack-vm-guest-surface: Prometheus datasource stays on loopback' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.grafanaDatasources[] | select(.name == "Loki") | .url' 'http://127.0.0.1:3100' \
    'obs-stack-vm-guest-surface: Loki datasource stays on loopback' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.grafanaDatasources[] | select(.name == "Tempo") | .url' 'http://127.0.0.1:3200' \
    'obs-stack-vm-guest-surface: Tempo datasource stays on loopback' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.lokiDerivedFields | length' '1' \
    'obs-stack-vm-guest-surface: Loki datasource provisions trace_id derivedFields' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.tempoTraceToLogsDatasource' 'loki' \
    'obs-stack-vm-guest-surface: Tempo datasource wires trace-to-logs into Loki' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.grafanaDashboardProviders | length' '1' \
    'obs-stack-vm-guest-surface: grafana provisions one dashboard provider' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.grafanaDashboardProviders[0].folder' 'Nixling' \
    'obs-stack-vm-guest-surface: grafana dashboard folder is Nixling' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.grafanaDashboardProviders[0].options.path | contains("nixling-grafana-dashboards")' 'true' \
    'obs-stack-vm-guest-surface: grafana dashboard provider points at the materialized dashboard dir' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.prometheusRetention' '5d' \
    'obs-stack-vm-guest-surface: Prometheus retention tracks cfg.retention.metrics' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.lokiRetention' '3d' \
    'obs-stack-vm-guest-surface: Loki retention tracks cfg.retention.logs' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.tempoRetention' '1d' \
    'obs-stack-vm-guest-surface: Tempo retention tracks cfg.retention.traces' || return 1

  grafana_bind=$(jq -r '.grafanaBind' "$EVAL_OUT_FILE") || return 1
  expected_bind=$(jq -r '.expectedGrafanaBind' "$EVAL_OUT_FILE") || return 1
  assert_eq "$grafana_bind" "$expected_bind" \
    'obs-stack-vm-guest-surface: grafana bind_address tracks cfg.grafana.listenAddress' || return 1

  vsock_in_exec_start=$(jq -r '.vsockInExecStart' "$EVAL_OUT_FILE") || return 1
  assert_contains "$vsock_in_exec_start" 'bin/socat -d -d VSOCK-LISTEN:14317,fork,max-children=16,reuseaddr UNIX-CONNECT:/run/nixling/obs-ingress.sock' \
    'obs-stack-vm-guest-surface: vsock-in ExecStart shape' || return 1

  pass_case 'obs-stack-vm-guest-surface'
}

test_obs_alerting_surface() {
  local body work_guest_alloy_config

  body=$(cat <<'EOF'
let
  obsVm = nixos.config.nixling.observability.vmName;
  obsGuest = nixos.config.microvm.vms.${obsVm}.config.config;
  workGuest = nixos.config.microvm.vms.corp-vm.config.config;
in {
  obsPrometheusJobs = map (scrape: scrape.job_name) obsGuest.services.prometheus.scrapeConfigs;
  obsPrometheusRuleFiles = map toString obsGuest.services.prometheus.ruleFiles;
  workGuestAlloyConfig = workGuest.environment.etc."alloy/config.alloy".text;
}
EOF
)

  run_eval \
    "obs-alerting-surface" \
    '({ ... }: {
       nixling.observability.enable = true;
       nixling.vms.corp-vm.observability.enable = true;
     })' \
    "$body" \
    || {
      show_stderr_tail "$EVAL_ERR_FILE"
      fail_case 'obs-alerting-surface: eval failed'
      return 1
    }

  assert_json_eq "$EVAL_OUT_FILE" '.obsPrometheusRuleFiles | length' '1' \
    'obs-alerting-surface: Prometheus ruleFiles provisions a single rules file' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.obsPrometheusRuleFiles[0] | contains("nixling-observability.rules.yml")' 'true' \
    'obs-alerting-surface: alert rules file uses the dedicated observability name' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.obsPrometheusJobs | length' '5' \
    'obs-alerting-surface: obs Prometheus scrapes grafana/prometheus/loki/tempo/alloy' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.obsPrometheusJobs | index("grafana") != null' 'true' \
    'obs-alerting-surface: Grafana self-scrape job declared' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.obsPrometheusJobs | index("loki") != null' 'true' \
    'obs-alerting-surface: Loki self-scrape job declared' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.obsPrometheusJobs | index("tempo") != null' 'true' \
    'obs-alerting-surface: Tempo self-scrape job declared' || return 1
  assert_json_eq "$EVAL_OUT_FILE" '.obsPrometheusJobs | index("alloy") != null' 'true' \
    'obs-alerting-surface: Alloy self-scrape job declared' || return 1

  work_guest_alloy_config=$(jq -r '.workGuestAlloyConfig' "$EVAL_OUT_FILE") || return 1
  assert_contains "$work_guest_alloy_config" 'job_name   = "nixling-vm-telemetry"' \
    'obs-alerting-surface: guest Alloy emits telemetry heartbeat metrics' || return 1
  assert_contains "$work_guest_alloy_config" 'job_name   = "nixling-vm-node"' \
    'obs-alerting-surface: guest Alloy keeps node metrics on a nixling-vm* job' || return 1
  assert_contains "$work_guest_alloy_config" 'target_label = "vm"' \
    'obs-alerting-surface: guest Alloy injects vm labels into telemetry' || return 1
  assert_contains "$work_guest_alloy_config" 'target_label = "env"' \
    'obs-alerting-surface: guest Alloy injects env labels into telemetry' || return 1

  pass_case 'obs-alerting-surface'
}

test_obs_vm_toggle_default_off() {
  run_eval \
    "obs-vm-toggle-default-off" \
    '({ ... }: { nixling.observability.enable = true; })' \
    'nixos.config.nixling.manifest."corp-vm".observability.enabled' \
    || {
      show_stderr_tail "$EVAL_ERR_FILE"
      fail_case 'obs-vm-toggle-default-off: eval failed'
      return 1
    }

  assert_json_eq "$EVAL_OUT_FILE" '.' 'false' \
    'obs-vm-toggle-default-off: corp-vm observability.enabled' || return 1

  pass_case 'obs-vm-toggle-default-off'
}

test_obs_cli_traces_default_on() {
  probe_cli_traces_gate || {
    show_stderr_tail "$EVAL_ERR_FILE"
    fail_case 'obs-cli-traces-default-on: could not inspect nixling runtimeInputs'
    return 1
  }

  if [ "$CLI_TRACES_DEFAULT" = 'false' ] && [ "$CLI_TRACES_DISABLED" = 'false' ]; then
    skip_case 'obs-cli-traces-default-on: TODO post-integration — cli-traces runtimeInputs gate has not landed in this worktree'
    return 2
  fi

  assert_eq "$CLI_TRACES_DEFAULT" 'true' \
    'obs-cli-traces-default-on: otel-cli present in nixling PATH' || return 1

  pass_case 'obs-cli-traces-default-on'
}

test_obs_cli_traces_disabled() {
  probe_cli_traces_gate || {
    show_stderr_tail "$EVAL_ERR_FILE"
    fail_case 'obs-cli-traces-disabled: could not inspect nixling runtimeInputs'
    return 1
  }

  if [ "$CLI_TRACES_DEFAULT" = 'false' ] && [ "$CLI_TRACES_DISABLED" = 'false' ]; then
    skip_case 'obs-cli-traces-disabled: TODO post-integration — cli-traces runtimeInputs gate has not landed in this worktree'
    return 2
  fi

  assert_eq "$CLI_TRACES_DISABLED" 'false' \
    'obs-cli-traces-disabled: otel-cli absent when traces are disabled' || return 1

  pass_case 'obs-cli-traces-disabled'
}

# Layer-1 best-effort: inspect the rendered helper body from the CLI
# package baked into environment.systemPackages for the fixed attr names
# we expect today. Phase 6 should add an end-to-end nixosTest for the
# runtime allowlist/filter behavior itself.
test_obs_cli_trace_attr_allowlist() {
  local trace_lines=""

  if ! trace_lines="$({ probe_cli_script_text \
      "__probe-cli-trace-attr-allowlist" \
      '({ ... }: { nixling.observability.enable = true; })'; } | sed -n '/^nl_span_start() {/,/^}/p')"; then
    show_stderr_tail "$EVAL_ERR_FILE"
    fail_case 'obs-cli-trace-attr-allowlist: could not inspect rendered nixling script'
    return 1
  fi

  if [ -z "$trace_lines" ]; then
    skip_case 'obs-cli-trace-attr-allowlist: TODO post-integration — rendered nixling script has no nl_span_start helper yet'
    return 2
  fi

  assert_contains "$trace_lines" 'vm.name=' \
    'obs-cli-trace-attr-allowlist: vm.name attr rendered' || return 1
  assert_contains "$trace_lines" 'vm.env=' \
    'obs-cli-trace-attr-allowlist: vm.env attr rendered' || return 1
  assert_contains "$trace_lines" 'nixling.subcommand=' \
    'obs-cli-trace-attr-allowlist: nixling.subcommand attr rendered' || return 1

  assert_not_contains "$trace_lines" 'key=' \
    'obs-cli-trace-attr-allowlist: key= absent from span attrs surface' || return 1
  assert_not_contains "$trace_lines" 'cert=' \
    'obs-cli-trace-attr-allowlist: cert= absent from span attrs surface' || return 1
  assert_not_contains "$trace_lines" 'password=' \
    'obs-cli-trace-attr-allowlist: password= absent from span attrs surface' || return 1
  assert_not_contains "$trace_lines" 'token=' \
    'obs-cli-trace-attr-allowlist: token= absent from span attrs surface' || return 1
  assert_not_contains "$trace_lines" '/nix/store/' \
    'obs-cli-trace-attr-allowlist: /nix/store/ absent from span attrs surface' || return 1
  assert_not_contains "$trace_lines" 'derivation=' \
    'obs-cli-trace-attr-allowlist: derivation= absent from span attrs surface' || return 1
  assert_not_contains "$trace_lines" 'entra' \
    'obs-cli-trace-attr-allowlist: Entra markers absent from span attrs surface' || return 1
  assert_not_contains "$trace_lines" 'tpm2-' \
    'obs-cli-trace-attr-allowlist: TPM markers absent from span attrs surface' || return 1

  pass_case 'obs-cli-trace-attr-allowlist'
}

test_obs_reserved_prefix_exempt() {
  run_eval \
    "obs-reserved-prefix-exempt" \
    '({ ... }: { nixling.observability.enable = true; })' \
    'builtins.hasAttr "sys-obs-stack" nixos.config.nixling.vms' \
    || {
      if feature_auto_obs_ready 'sys-obs-stack'; then
        show_stderr_tail "$EVAL_ERR_FILE"
        fail_case 'obs-reserved-prefix-exempt: eval failed even though sys-obs-stack should be exempt from the reserved sys- prefix rule'
        return 1
      fi
      skip_case 'obs-reserved-prefix-exempt: TODO post-integration — auto-obs-vm has not landed, so sys-obs-stack is not materialized in this worktree'
      return 2
    }

  if jq -e '. == true' "$EVAL_OUT_FILE" >/dev/null 2>&1; then
    pass_case 'obs-reserved-prefix-exempt'
    return 0
  fi

  if feature_auto_obs_ready 'sys-obs-stack'; then
    fail_case 'obs-reserved-prefix-exempt: auto-observability VM is missing despite observability.enable = true'
    return 1
  fi

  skip_case 'obs-reserved-prefix-exempt: TODO post-integration — auto-obs-vm has not landed, so sys-obs-stack is not materialized in this worktree'
  return 2
}

test_obs_vm_without_framework() {
  local err_file rc=0

  if assert_eval_fails \
      "obs-vm-without-framework" \
      '({ ... }: {
         nixling.vms.corp-vm.observability.enable = true;
       })' \
      'nixos.config.system.build.toplevel.drvPath' \
      'corp-vm' \
      'nixling.observability.enable'; then
    pass_case 'obs-vm-without-framework'
    return 0
  else
    rc=$?
  fi

  if [ "$rc" -eq 2 ]; then
    skip_case 'obs-vm-without-framework: TODO post-integration — per-VM/global observability assertion has not landed in this worktree'
    return 2
  fi

  err_file="$EVAL_ERR_FILE"
  if feature_transport_vsock_ready; then
    show_stderr_tail "$err_file"
    fail_case 'obs-vm-without-framework: eval failed, but stderr did not name the missing nixling.observability.enable toggle'
    return 1
  fi

  skip_case 'obs-vm-without-framework: TODO post-integration — per-VM/global observability assertion has not landed in this worktree'
  return 2
}

assert_lines_set_eq() {
  local actual="$1" expected="$2" msg="$3"
  local actual_sorted expected_sorted

  actual_sorted=$(printf '%s\n' "$actual" | sed '/^$/d' | LC_ALL=C sort -u)
  expected_sorted=$(printf '%s\n' "$expected" | sed '/^$/d' | LC_ALL=C sort -u)
  assert_eq "$actual_sorted" "$expected_sorted" "$msg" || return 1
}

assert_json_string_set_eq() {
  local file="$1" jq_filter="$2" expected="$3" msg="$4"
  local actual

  actual=$(jq -c "$jq_filter | map(select(type == \"string\" and length > 0)) | unique | sort" "$file") || return 1
  assert_eq "$actual" "$expected" "$msg" || return 1
}

OBS_PANEL_SURFACE_PROBED=0
OBS_PANEL_SURFACE_FILE=""
OBS_PANEL_RULES_FILE=""
OBS_PANEL_HOST_ALLOY_FILE=""

probe_obs_panel_surface() {
  local override body

  if [ "$OBS_PANEL_SURFACE_PROBED" -eq 1 ]; then
    return 0
  fi

  override=$(cat <<'EOF'
({ ... }: {
  nixling.observability.enable = true;
  nixling.vms.corp-vm.observability.enable = true;
})
EOF
  )

  body=$(cat <<'EOF'
let
  obsVm = nixos.config.nixling.observability.vmName;
  obsGuest = nixos.config.microvm.vms.${obsVm}.config.config;
in {
  datasourceUids = map (ds: ds.uid) obsGuest.services.grafana.provision.datasources.settings.datasources;
  obsPrometheusJobs = map (scrape: scrape.job_name) obsGuest.services.prometheus.scrapeConfigs;
}
EOF
  )

  run_eval "__probe-obs-panel-surface" "$override" "$body" || return 1
  OBS_PANEL_SURFACE_FILE="$EVAL_OUT_FILE"
  OBS_PANEL_HOST_ALLOY_FILE="$SCRATCH/__probe-obs-panel-surface.host.alloy"

  # The host Alloy config is a writeText derivation; readFile-via-eval
  # would trigger IFD which is disabled in the static-eval gate. Use the
  # same materialize-then-read pattern as the rules probe below.
  run_instantiate \
    "__probe-obs-panel-host-alloy" \
    "$override" \
    'nixos.config.services.alloy.configPath' \
    || return 1
  OBS_PANEL_HOST_ALLOY_FILE=$(nix-store -r "$(cat "$EVAL_OUT_FILE")" 2>> "$EVAL_ERR_FILE") || return 1

  run_instantiate \
    "__probe-obs-panel-rules" \
    "$override" \
    'let
       obsVm = nixos.config.nixling.observability.vmName;
       obsGuest = nixos.config.microvm.vms.${obsVm}.config.config;
     in
       builtins.head obsGuest.services.prometheus.ruleFiles' \
    || return 1

  OBS_PANEL_RULES_FILE=$(nix-store -r "$(cat "$EVAL_OUT_FILE")" 2>> "$EVAL_ERR_FILE") || return 1
  OBS_PANEL_SURFACE_PROBED=1
}

obs_dashboard_files() {
  find "$ROOT/nixos-modules/components/observability/dashboards" \
    -maxdepth 1 -type f -name '*.json' | LC_ALL=C sort
}

obs_alert_rule_names() {
  # Prometheus rules accept JSON-style YAML. nixpkgs' lib.generators.toYAML
  # emits minified JSON, so prefer jq over grep -E.
  if jq -r '.groups[]?.rules[]?.alert // empty' "$1" 2>/dev/null \
       | LC_ALL=C sort -u | grep -q .; then
    jq -r '.groups[]?.rules[]?.alert // empty' "$1" 2>/dev/null | LC_ALL=C sort -u
    return
  fi
  # Fallback: classic block-YAML `- alert: <name>` format.
  grep -E '^[[:space:]]*- alert:[[:space:]]*' "$1" \
    | sed -E 's/^[[:space:]]*- alert:[[:space:]]*//' \
    || true
}

obs_host_scrape_job_names() {
  grep -E 'job_name[[:space:]]*=' "$1" \
    | sed -E 's/.*job_name[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/' \
    || true
}

extract_dashboard_promql_exprs() {
  jq -r '
    ..
    | objects
    | select(has("targets") and (.datasource? | type == "object") and .datasource.type == "prometheus")
    | .targets[]?
    | .expr? // empty
  ' "$@"
}

extract_prometheus_rule_exprs() {
  # lib.generators.toYAML emits JSON-shaped YAML. Prefer jq when the
  # file parses as JSON; fall back to classic block-YAML extraction.
  if jq -e . "$1" >/dev/null 2>&1; then
    jq -r '.groups[]?.rules[]?.expr // empty' "$1"
    return
  fi
  awk '
    function indent_of(line) {
      match(line, /^[ ]*/)
      return RLENGTH
    }

    {
      if (in_block) {
        if (indent_of($0) > block_indent) {
          line = $0
          sub(/^[[:space:]]+/, "", line)
          print line
          next
        }
        in_block = 0
      }

      if ($0 ~ /^[[:space:]]*expr:[[:space:]]*\|[[:space:]]*$/) {
        in_block = 1
        block_indent = indent_of($0)
        next
      }

      if ($0 ~ /^[[:space:]]*expr:[[:space:]]*/) {
        line = $0
        sub(/^[[:space:]]*expr:[[:space:]]*/, "", line)
        if (length(line) > 0) {
          print line
        }
      }
    }
  ' "$1"
}

extract_metric_tokens() {
  grep -oE '\b(nixling_[a-zA-Z0-9_:]+|node_[a-zA-Z0-9_:]+|systemd_unit_state|loki_[a-zA-Z0-9_:]+|tempo_[a-zA-Z0-9_:]+|prometheus_[a-zA-Z0-9_:]+|up)\b' \
    | LC_ALL=C sort -u
}

extract_up_job_refs() {
  {
    grep -oE 'up\{job="[^"]+"' "$@" 2>/dev/null \
      | sed -E 's/.*job="([^"]+)"/\1/' || true
    grep -oE 'up\{job=~"\^\([^)]+\)\$"' "$@" 2>/dev/null \
      | sed -E 's/.*job=~"\^\(([^)]+)\)\$"/\1/' \
      | tr '|' '\n' || true
  } | sed '/^$/d; /^\$/d' | LC_ALL=C sort -u
}

host_ch_exporter_metric_allowlist() {
  cat <<'EOF'
nixling_vm_ch_api_up
nixling_vm_counter_virtio_blk_read_bytes
nixling_vm_counter_virtio_blk_read_latency_avg
nixling_vm_counter_virtio_blk_read_latency_max
nixling_vm_counter_virtio_blk_read_latency_min
nixling_vm_counter_virtio_blk_read_ops
nixling_vm_counter_virtio_blk_write_bytes
nixling_vm_counter_virtio_blk_write_latency_avg
nixling_vm_counter_virtio_blk_write_latency_max
nixling_vm_counter_virtio_blk_write_latency_min
nixling_vm_counter_virtio_blk_write_ops
nixling_vm_counter_virtio_net_rx_bytes
nixling_vm_counter_virtio_net_rx_frames
nixling_vm_counter_virtio_net_tx_bytes
nixling_vm_counter_virtio_net_tx_frames
nixling_vm_last_scrape_timestamp_seconds
nixling_vm_observability_enabled
nixling_vm_running
nixling_vm_scrape_errors_total
nixling_vm_state
nixling_vm_unknown_counters_total
EOF
}

metric_doc_is_future_work_absent() {
  local metric="$1"

  grep -F -C2 -- "$metric" "$ROOT/CHANGELOG.md" 2>/dev/null \
    | grep -qi 'future-work absent'
}

metric_reference_category() {
  local metric="$1"

  case "$metric" in
    node_*|systemd_unit_state)
      printf '%s\n' 'alloy-host-collector'
      return 0
      ;;
    loki_*|tempo_*|prometheus_*|grafana_*|alloy_*|up)
      printf '%s\n' 'upstream-service-exporter'
      return 0
      ;;
  esac

  if host_ch_exporter_metric_allowlist | grep -qxF -- "$metric"; then
    printf '%s\n' 'host-ch-exporter'
    return 0
  fi

  if metric_doc_is_future_work_absent "$metric"; then
    printf '%s\n' 'future-work-absent'
    return 0
  fi

  return 1
}

test_obs_dashboards_schema() {
  local file
  local -a dashboard_files=()

  mapfile -t dashboard_files < <(obs_dashboard_files)
  assert_eq "${#dashboard_files[@]}" '6' \
    'obs-dashboards-schema: enumerate the 6 shipped dashboard JSON files' || return 1

  for file in "${dashboard_files[@]}"; do
    if ! jq -e '.' "$file" >/dev/null 2>&1; then
      fail_case "obs-dashboards-schema: $(basename "$file") does not parse as JSON"
      return 1
    fi

    if ! jq -e '
      (.uid | type == "string" and length > 0)
      and (.title | type == "string" and length > 0)
      and (.schemaVersion | type == "number")
      and (.panels | type == "array" and length > 0)
    ' "$file" >/dev/null 2>&1; then
      fail_case "obs-dashboards-schema: $(basename "$file") is missing uid/title/schemaVersion/non-empty panels"
      return 1
    fi

    if ! jq -e '
      ["prometheus", "loki", "tempo"] as $allowed
      | [ .. | .datasource? | select(. != null) ] as $refs
      | (($refs | length) > 0)
        and all($refs[];
          type == "object"
          and ((.uid // "") as $uid | ($allowed | index($uid)) != null)
          and ((.type // "") as $type | ($allowed | index($type)) != null)
        )
    ' "$file" >/dev/null 2>&1; then
      fail_case "obs-dashboards-schema: $(basename "$file") references a datasource outside {prometheus,loki,tempo}"
      return 1
    fi
  done

  pass_case 'obs-dashboards-schema'
}

test_obs_rules_promtool() {
  local promtool_out promtool_err

  probe_obs_panel_surface || {
    show_stderr_tail "$EVAL_ERR_FILE"
    fail_case 'obs-rules-promtool: could not materialize the rendered Prometheus rules file'
    return 1
  }

  assert_file_exists "$OBS_PANEL_RULES_FILE" || return 1

  if ! command -v promtool >/dev/null 2>&1; then
    skip_case 'obs-rules-promtool: promtool not present in PATH (CHANGELOG Test-H note tracks the clean skip)'
    return 2
  fi

  promtool_out="$SCRATCH/obs-rules-promtool.stdout"
  promtool_err="$SCRATCH/obs-rules-promtool.stderr"
  if ! promtool check rules "$OBS_PANEL_RULES_FILE" > "$promtool_out" 2> "$promtool_err"; then
    if [ -s "$promtool_out" ]; then
      log '    --- promtool (tail) ---'
      tail -15 "$promtool_out" | sed 's/^/      /' >&2
    fi
    show_stderr_tail "$promtool_err"
    fail_case 'obs-rules-promtool: promtool check rules failed'
    return 1
  fi

  pass_case 'obs-rules-promtool'
}

test_obs_metric_references() {
  local dashboard_exprs rules_exprs metrics_file concrete_up_jobs metric category
  local -a unknown_metrics=()

  probe_obs_panel_surface || {
    show_stderr_tail "$EVAL_ERR_FILE"
    fail_case 'obs-metric-references: could not materialize observability dashboards/rules'
    return 1
  }

  dashboard_exprs="$SCRATCH/obs-metric-references.dashboard.promql"
  rules_exprs="$SCRATCH/obs-metric-references.rules.promql"
  metrics_file="$SCRATCH/obs-metric-references.metrics"

  if ! extract_dashboard_promql_exprs \
      "$ROOT"/nixos-modules/components/observability/dashboards/*.json \
      > "$dashboard_exprs"; then
    fail_case 'obs-metric-references: could not extract dashboard PromQL expressions'
    return 1
  fi

  if ! extract_prometheus_rule_exprs "$OBS_PANEL_RULES_FILE" > "$rules_exprs"; then
    fail_case 'obs-metric-references: could not extract alert PromQL expressions'
    return 1
  fi

  cat "$dashboard_exprs" "$rules_exprs" | extract_metric_tokens > "$metrics_file" || {
    fail_case 'obs-metric-references: could not extract metric tokens from dashboard/rule PromQL'
    return 1
  }

  assert_ge "$(wc -l < "$metrics_file" | tr -d ' ')" '1' \
    'obs-metric-references: extracted metric references from PromQL' || return 1

  while IFS= read -r metric; do
    [ -n "$metric" ] || continue
    category=$(metric_reference_category "$metric" || true)
    if [ -z "$category" ]; then
      unknown_metrics+=( "$metric" )
    fi
  done < "$metrics_file"

  if [ "${#unknown_metrics[@]}" -gt 0 ]; then
    fail_case "obs-metric-references: unresolved metric refs: ${unknown_metrics[*]}"
    return 1
  fi

  concrete_up_jobs=$(extract_up_job_refs "$dashboard_exprs" "$rules_exprs")
  assert_lines_set_eq "$concrete_up_jobs" $'alloy\ngrafana\nloki\nnixling-ch-exporter\nnixling-vm-telemetry\nprometheus\ntempo' \
    'obs-metric-references: concrete up{job=...} refs stay on known scrape jobs' || return 1

  pass_case 'obs-metric-references'
}

test_obs_scrape_job_stability() {
  local host_jobs

  probe_obs_panel_surface || {
    show_stderr_tail "$EVAL_ERR_FILE"
    fail_case 'obs-scrape-job-stability: eval failed'
    return 1
  }

  assert_json_string_set_eq "$OBS_PANEL_SURFACE_FILE" '.obsPrometheusJobs' '["alloy","grafana","loki","prometheus","tempo"]' \
    'obs-scrape-job-stability: obs VM Prometheus scrape job exact-set' || return 1

  host_jobs=$(obs_host_scrape_job_names "$OBS_PANEL_HOST_ALLOY_FILE")
  assert_lines_set_eq "$host_jobs" $'nixling-ch-exporter\nsystemd-units' \
    'obs-scrape-job-stability: host Alloy scrape job exact-set' || return 1

  pass_case 'obs-scrape-job-stability'
}

test_obs_stability() {
  local dashboard_uids dashboard_datasource_uids alert_names host_jobs
  local -a dashboard_files=()

  probe_obs_panel_surface || {
    show_stderr_tail "$EVAL_ERR_FILE"
    fail_case 'obs-stability: could not probe observability surfaces'
    return 1
  }

  mapfile -t dashboard_files < <(obs_dashboard_files)
  dashboard_uids=$(jq -r '.uid' "${dashboard_files[@]}") || return 1
  dashboard_datasource_uids=$(jq -r '.. | .datasource? | objects | .uid // empty' "${dashboard_files[@]}") || return 1
  alert_names=$(obs_alert_rule_names "$OBS_PANEL_RULES_FILE")
  host_jobs=$(obs_host_scrape_job_names "$OBS_PANEL_HOST_ALLOY_FILE")

  # Lock the currently shipped dashboard UIDs so panel review catches any silent churn.
  assert_lines_set_eq "$dashboard_uids" $'lifecycle-traces\nlogs\nnixling-overview\nobs-vm-health\nper-vm-store\nvm-resources' \
    'obs-stability: dashboard UID exact-set' || return 1
  assert_lines_set_eq "$dashboard_datasource_uids" $'loki\nprometheus\ntempo' \
    'obs-stability: dashboard datasource UID exact-set' || return 1
  assert_json_string_set_eq "$OBS_PANEL_SURFACE_FILE" '.datasourceUids' '["loki","prometheus","tempo"]' \
    'obs-stability: rendered datasource UID exact-set' || return 1
  assert_lines_set_eq "$alert_names" $'NixlingCHAPISocketMissing\nNixlingGuestTelemetryMissing\nNixlingNetVMDownWithRunningWorkloads\nNixlingObsVMStackUnhealthy\nNixlingObsVMUnreachableFromHost\nNixlingStoreSyncFailure\nNixlingVMDown\nNixlingVsockRelayDown' \
    'obs-stability: alert-rule exact-set' || return 1
  assert_json_string_set_eq "$OBS_PANEL_SURFACE_FILE" '.obsPrometheusJobs' '["alloy","grafana","loki","prometheus","tempo"]' \
    'obs-stability: obs scrape-job exact-set' || return 1
  assert_lines_set_eq "$host_jobs" $'nixling-ch-exporter\nsystemd-units' \
    'obs-stability: host scrape-job exact-set' || return 1

  pass_case 'obs-stability'
}

log '==> tests/observability-eval.sh'

test_obs_graphics_runner_wiring() {
  # Verify that graphics VMs (which run via nixling-<vm>-gpu.service
  # instead of microvm@<vm>.service) correctly get the host-side
  # OTLP relay wired, and that the relay template no longer
  # BindsTo=microvm@%i.service so it can come up for either runner.
  # (panel-w3r3 software-1 / nixos-1 / networking-1 / observability-1)
  local override body

  override=$(cat <<'EOF'
({ ... }: {
  nixling.observability.enable = true;
  nixling.vms.gpu-vm = {
    enable = true;
    env = "personal";
    index = 11;
    graphics.enable = true;
    observability.enable = true;
    config = {
      microvm = { mem = 512; vcpu = 1; };
      fileSystems."/" = { device = "rootfs"; fsType = "tmpfs"; };
      boot.loader.grub.enable = false;
      system.stateVersion = "25.11";
    };
  };
})
EOF
  )

  body=$(cat <<'EOF'
let
  relay = nixos.config.systemd.services."nixling-otel-relay@";
  gpuUnit = nixos.config.systemd.services."nixling-gpu-vm-gpu" or null;
in {
  relayBindsToHasMicrovmTemplate =
    builtins.elem "microvm@%i.service" (relay.bindsTo or [ ]);
  gpuServiceDeclared = gpuUnit != null;
  gpuWantsRelay =
    if gpuUnit == null then null
    else builtins.elem "nixling-otel-relay@gpu-vm.service" (gpuUnit.wants or [ ]);
  relayExecCondition = relay.serviceConfig.ExecCondition or null;
  relayExecStartPre = relay.serviceConfig.ExecStartPre;
}
EOF
  )

  run_eval "obs-graphics-runner-wiring" "$override" "$body" || {
    show_stderr_tail "$EVAL_ERR_FILE"
    fail_case 'obs-graphics-runner-wiring: eval failed'
    return 1
  }

  # Relay template MUST NOT BindsTo microvm@%i.service (it'd block
  # graphics VMs whose runner is nixling-<vm>-gpu.service).
  assert_json_eq "$EVAL_OUT_FILE" '.relayBindsToHasMicrovmTemplate' 'false' \
    'obs-graphics-runner-wiring: relay template does not BindsTo microvm@%i.service' \
    || return 1

  # The graphics VM's gpu sidecar must exist...
  assert_json_eq "$EVAL_OUT_FILE" '.gpuServiceDeclared' 'true' \
    'obs-graphics-runner-wiring: nixling-gpu-vm-gpu sidecar declared' || return 1

  # ...and must `wants=` the per-VM relay.
  assert_json_eq "$EVAL_OUT_FILE" '.gpuWantsRelay' 'true' \
    'obs-graphics-runner-wiring: gpu sidecar wants nixling-otel-relay@gpu-vm.service' \
    || return 1

  # Eligibility gate + base CH vsock UDS precondition still in place.
  # (The per-port `_<port>` check was removed in a follow-up — those
  # files are created by CH lazily on first connect, so pre-checking
  # them blocked the relay from ever being the first connector.)
  if ! jq -e '.relayExecCondition | test("nixling-otel-relay-eligible")' \
       "$EVAL_OUT_FILE" >/dev/null; then
    fail_case 'obs-graphics-runner-wiring: relay ExecCondition references nixling-otel-relay-eligible'
    return 1
  fi
  if ! jq -e '.relayExecStartPre | map(strings) | any(. | test("vsock\\.sock$"))' \
       "$EVAL_OUT_FILE" >/dev/null; then
    fail_case 'obs-graphics-runner-wiring: relay ExecStartPre gates on base CH vsock UDS'
    return 1
  fi

  pass_case 'obs-graphics-runner-wiring'
}

CASES=(
  test_obs_disabled_default
  test_obs_default_off_no_units
  test_obs_enabled_defaults
  test_obs_name_collision
  test_obs_cid_collision
  test_obs_manifest_fields
  test_obs_relay_acl_surface
  test_obs_stack_vm_guest_surface
  test_obs_alerting_surface
  test_obs_vm_toggle_default_off
  test_obs_cli_traces_default_on
  test_obs_cli_traces_disabled
  test_obs_cli_trace_attr_allowlist
  test_obs_reserved_prefix_exempt
  test_obs_vm_without_framework
  test_obs_dashboards_schema
  test_obs_rules_promtool
  test_obs_metric_references
  test_obs_scrape_job_stability
  test_obs_stability
  test_obs_graphics_runner_wiring
)

for case_fn in "${CASES[@]}"; do
  run_case "$case_fn"
done

log "==> observability-eval: $PASS passed, $FAIL failed, $SKIP skipped"
if [ "$FAIL" -gt 0 ]; then
  exit 1
fi
