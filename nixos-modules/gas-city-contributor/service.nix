{ config, lib, pkgs, ... }:

let
  cfg = config.services.gasCityContributor;
  buildBuddyEnabled =
    cfg.buildBuddy.enable || cfg.credentials.buildBuddyApiKeyFile != null;
  package = cfg.package;
  python = "${package}/bin/python3";
  activation = "${package}/share/gas-city-contributor/pack/scripts/service-activation.py";
  discordDecision = "${package}/share/gas-city-contributor/pack/scripts/discord-decision.py";
  publisher = "${package}/share/gas-city-contributor/pack/scripts/publish-pr.py";
  launcher = "${package}/share/gas-city-contributor/pack/scripts/agent-launcher.py";
  sandbox = "${package}/share/gas-city-contributor/pack/scripts/agent-sandbox.py";
  fdproxy = "${package}/share/gas-city-contributor/pack/scripts/fdproxy.py";
  copilot = "${package}/bin/copilot";
  bwrap = "${package}/bin/bwrap";
  serviceRoot = "/var/lib/gascity-contributor";
  stateRoot = "${serviceRoot}/state";
  cancellationRoot = "${stateRoot}/cancellations";
  discordStateRoot = "/var/lib/gascity-discord";
  publisherStateRoot = "/var/lib/gascity-publisher";
  cacheRoot = "/var/cache/gascity-contributor";
  runtimeRoot = "/run/gascity-contributor";
  readinessPath = "${runtimeRoot}/readiness.json";
  egressSocket = "${runtimeRoot}/egress.sock";
  discordSocket = "${runtimeRoot}/discord.sock";
  publisherSocket = "${runtimeRoot}/publisher.sock";
  checkSocket = "${runtimeRoot}/check.sock";
  agentSocket = "${runtimeRoot}/agent.sock";
  agentPrivateSocket = "/run/gascity-agent/agent.sock";
  generation = builtins.substring 0 32 (builtins.hashString "sha256" (toString package));
  relayAuth = builtins.hashString "sha256"
    "gascity-fdproxy:${cfg.repository.githubSlug}:${cfg.repository.rigName}";
  checkAuth = builtins.hashString "sha256"
    "gascity-check:${cfg.repository.githubSlug}:${cfg.repository.rigName}";
  sidecarProxyEnvironment = [
    "HTTP_PROXY=http://127.0.0.1:3128"
    "HTTPS_PROXY=http://127.0.0.1:3128"
    "http_proxy=http://127.0.0.1:3128"
    "https_proxy=http://127.0.0.1:3128"
    "NO_PROXY="
    "no_proxy="
  ];
  projectionArgs = lib.concatMapStringsSep " " (
    path: "--projection ${lib.escapeShellArg path}"
  ) cfg.hostReadOnlyPaths;
  credentialArgs = (lib.concatStringsSep " " [
    "--credential ${lib.escapeShellArg cfg.credentials.copilotTokenFile}"
    "--credential ${lib.escapeShellArg cfg.credentials.githubPrivateKeyFile}"
    "--credential ${lib.escapeShellArg cfg.credentials.discordBotTokenFile}"
  ]) + lib.optionalString (cfg.credentials.buildBuddyApiKeyFile != null)
    " --credential ${lib.escapeShellArg cfg.credentials.buildBuddyApiKeyFile}";
  validatePaths = "+${python} ${activation} validate-paths"
    + " --project-root ${lib.escapeShellArg serviceRoot}"
    + " --require-project-quota"
    + " ${credentialArgs} ${projectionArgs}";
  checkReserve = "+${python} ${activation} check-free-space"
    + " --path ${lib.escapeShellArg serviceRoot}"
    + " --reserve-bytes ${toString cfg.storage.minFreeBytes}";
  materialize = "+${python} ${activation} materialize-assets"
    + " --source ${lib.escapeShellArg "${package}/share/gas-city-contributor"}"
    + " --destination ${lib.escapeShellArg "${serviceRoot}/managed"}";
  decisionReconcile = pkgs.writeShellScript "gascity-decision-reconcile" ''
    set -euo pipefail
    reconcile="$(${python} ${discordDecision} reconcile \
      --socket ${lib.escapeShellArg discordSocket})"
    ${pkgs.jq}/bin/jq -c \
      '.[] | select(.state == "answered" or .state == "answer-pending")' \
      <<<"$reconcile" \
      | while IFS= read -r record; do
        bead_id="$(${pkgs.jq}/bin/jq -r '.bead_id' <<<"$record")"
        run_id="$(${pkgs.jq}/bin/jq -r '.run_id' <<<"$record")"
        decision_id="$(${pkgs.jq}/bin/jq -r '.decision_id' <<<"$record")"
        nonce="$(${pkgs.jq}/bin/jq -r '.prompt_nonce' <<<"$record")"
        message_id="$(${pkgs.jq}/bin/jq -r '.message_id' <<<"$record")"
        state="$(${pkgs.jq}/bin/jq -r '.state' <<<"$record")"
        assignee="$(${pkgs.jq}/bin/jq -r '.assignee // ""' <<<"$record")"
        if test "$state" = answer-pending; then
          event_id="$(${pkgs.jq}/bin/jq -r '.pending_answer.event_id' <<<"$record")"
          choice="$(${pkgs.jq}/bin/jq -r '.pending_answer.choice' <<<"$record")"
        else
          event_id="$(${pkgs.jq}/bin/jq -r '.event_id' <<<"$record")"
          choice="$(${pkgs.jq}/bin/jq -r '.answer' <<<"$record")"
        fi
        bead="$(${package}/bin/bd show "$bead_id" --json)"
        if ! ${pkgs.jq}/bin/jq -e \
          --arg run "$run_id" \
          --arg decision "$decision_id" \
          --arg nonce "$nonce" \
          --arg message "$message_id" \
          --arg event "$event_id" \
          --arg choice "$choice" '
            (if type == "array" then .[0] else . end) as $bead
            | (($bead.metadata // {}) as $metadata
              | ($metadata.decision_run_id // "") == $run
              and ($metadata.decision_id // "") == $decision
              and ($metadata.decision_nonce // "") == $nonce
              and ($metadata.decision_message_id // "") == $message
              and ($metadata.decision_event_id // "") == $event
              and ($metadata.decision_choice // "") == $choice)
          ' <<<"$bead" >/dev/null; then
          pending_status="$(${pkgs.jq}/bin/jq -r \
            '(if type == "array" then .[0] else . end).status // ""' <<<"$bead")"
          if test "$state" = answer-pending && test "$pending_status" = blocked; then
            continue
          fi
          echo "decision gate metadata diverged: $bead_id" >&2
          exit 1
        fi
        status="$(${pkgs.jq}/bin/jq -r \
          '(if type == "array" then .[0] else . end).status // ""' <<<"$bead")"
        if test "$status" = in_progress; then
          assignee_args=()
          if test -n "$assignee"; then
            assignee_args=(--if-assignee "$assignee")
          fi
          set +e
          ${package}/bin/bd update "$bead_id" \
            "''${assignee_args[@]}" \
            --if-status in_progress \
            --status closed
          close_status="$?"
          set -e
          if test "$close_status" -ne 0 && test "$close_status" -ne 13; then
            exit "$close_status"
          fi
          if test "$close_status" -eq 13; then
            bead="$(${package}/bin/bd show "$bead_id" --json)"
            if ! ${pkgs.jq}/bin/jq -e \
              '(if type == "array" then .[0] else . end).status == "closed"' \
              <<<"$bead" >/dev/null; then
              echo "decision gate close precondition was lost: $bead_id" >&2
              exit 13
            fi
          fi
        elif test "$status" != closed; then
          echo "answered decision gate is not open or closed: $bead_id" >&2
          exit 1
        fi
        if test "$state" = answer-pending; then
          ${python} ${discordDecision} ack \
            --socket ${lib.escapeShellArg discordSocket} \
            --run-id "$run_id" \
            --decision-id "$decision_id" \
            --event-id "$event_id" \
            --choice "$choice" \
            --accepted
        fi
        ${python} ${discordDecision} close \
          --socket ${lib.escapeShellArg discordSocket} \
          --run-id "$run_id" \
          --decision-id "$decision_id" \
          --event-id "$event_id" \
          --choice "$choice"
      done
  '';

  commonServiceConfig = {
    Slice = "gascity-contributor.slice";
    NoNewPrivileges = true;
    PrivateTmp = true;
    PrivateDevices = true;
    ProtectSystem = "strict";
    ProtectHome = true;
    ProtectKernelTunables = true;
    ProtectKernelModules = true;
    ProtectKernelLogs = true;
    ProtectControlGroups = true;
    ProtectClock = true;
    ProtectHostname = true;
    ProtectProc = "invisible";
    ProcSubset = "pid";
    RestrictSUIDSGID = true;
    RestrictRealtime = true;
    LockPersonality = true;
    CapabilityBoundingSet = [ "" ];
    AmbientCapabilities = [ "" ];
    UMask = "0077";
    RestrictAddressFamilies = [ "AF_UNIX" "AF_INET" "AF_INET6" ];
    InaccessiblePaths = [
      "-/etc/shadow"
      "-/etc/gshadow"
      "-/etc/ssh"
      "-/run/systemd"
      "-/nix/var/nix/daemon-socket/socket"
      "-/proc/kcore"
      "-/proc/keys"
      "-/proc/latency_stats"
    ];
    SystemCallFilter = [
      "@system-service"
      "~@privileged"
      "~@mount"
      "~@raw-io"
      "chown"
    ];
    Restart = "on-failure";
    RestartSec = "2s";
    KillMode = "control-group";
  };

  sharedServiceConfig = commonServiceConfig // {
    SupplementaryGroups = [ "gascity-contributor" ];
    ReadWritePaths = [ runtimeRoot ];
  };

  waitReadiness = pkgs.writeShellScript "gascity-wait-readiness" ''
    set -euo pipefail
    for _ in $(${pkgs.coreutils}/bin/seq 1 600); do
      if test -s ${lib.escapeShellArg readinessPath} \
        && test -S ${lib.escapeShellArg agentSocket} \
        && test -S ${lib.escapeShellArg discordSocket} \
        && test -S ${lib.escapeShellArg publisherSocket} \
        ${lib.optionalString cfg.check.enable
          "&& test -S ${lib.escapeShellArg checkSocket}"}; then
        exit 0
      fi
      sleep 0.5
    done
    echo "Gas City agent readiness did not become available" >&2
    exit 1
  '';

  agentStart = pkgs.writeShellScript "gascity-agent-start" ''
    set -euo pipefail
    umask 077
    bootstrap_pid=""
    launcher_pid=""
    relay_pid=""
    cleanup() {
      for pid in "$relay_pid" "$launcher_pid" "$bootstrap_pid"; do
        if test -n "$pid"; then
          kill "$pid" 2>/dev/null || true
        fi
      done
      for pid in "$relay_pid" "$launcher_pid" "$bootstrap_pid"; do
        if test -n "$pid"; then
          wait "$pid" 2>/dev/null || true
        fi
      done
    }
    trap cleanup EXIT TERM INT
    ${pkgs.coreutils}/bin/install -d -m 0700 ${lib.escapeShellArg "${runtimeRoot}/agent"}
    token="$CREDENTIALS_DIRECTORY/copilot-token"
    test -s "$token"
    export COPILOT_GITHUB_TOKEN="$(<"$token")"
    for _ in $(${pkgs.coreutils}/bin/seq 1 100); do
      test -S ${lib.escapeShellArg egressSocket} && break
      sleep 0.05
    done
    test -S ${lib.escapeShellArg egressSocket}
    ${python} ${launcher} \
      --server \
      --socket ${lib.escapeShellArg agentPrivateSocket} \
      --settings-root ${lib.escapeShellArg "${package}/share/gas-city-contributor/copilot"} \
      --copilot ${lib.escapeShellArg copilot} \
      --state-root ${lib.escapeShellArg "${stateRoot}/agent-state"} \
      --worktree ${lib.escapeShellArg "${stateRoot}/worktrees"} \
      --lease-root ${lib.escapeShellArg "${stateRoot}/leases"} \
      --runtime-root ${lib.escapeShellArg "${runtimeRoot}/agent"} \
      --runtime-path ${lib.escapeShellArg "${package}/bin"} \
      --runtime-path ${lib.escapeShellArg "${package}/share/gas-city-contributor"} \
      --sandbox-script ${lib.escapeShellArg sandbox} \
      --fdproxy-script ${lib.escapeShellArg fdproxy} \
      --sandbox-python ${lib.escapeShellArg python} \
      --bwrap-path ${lib.escapeShellArg bwrap} \
      --max-agents ${toString cfg.resources.maxConcurrentAgents} \
      --max-active-runs ${toString cfg.resources.maxActiveRuns} \
      --client-uid ${toString config.users.users.gascity-agent.uid} \
      --generation ${lib.escapeShellArg generation} \
      --state-schema 1 &
    bootstrap_pid="$!"
    for _ in $(${pkgs.coreutils}/bin/seq 1 100); do
      test -S ${lib.escapeShellArg agentPrivateSocket} && break
      kill -0 "$bootstrap_pid" 2>/dev/null || exit 1
      sleep 0.05
    done
    test -S ${lib.escapeShellArg agentPrivateSocket}
    ${pkgs.coreutils}/bin/install -d -m 0700 \
      ${lib.escapeShellArg "${stateRoot}/worktrees/readiness"}
    ${python} ${activation} activate \
      --status-path ${lib.escapeShellArg readinessPath} \
      --generation ${lib.escapeShellArg generation} \
      --state-schema 1 \
      --profile-script ${lib.escapeShellArg "${package}/share/gas-city-contributor/pack/scripts/copilot-profile.py"} \
      --run-id readiness \
      --bead-id readiness \
      --worktree ${lib.escapeShellArg "${stateRoot}/worktrees/readiness"} \
      --lease-root ${lib.escapeShellArg "${stateRoot}/leases"} \
      --runtime-root ${lib.escapeShellArg "${runtimeRoot}/agent"} \
      --egress-socket ${lib.escapeShellArg egressSocket}
    kill "$bootstrap_pid" 2>/dev/null || true
    wait "$bootstrap_pid" 2>/dev/null || true
    bootstrap_pid=""
    for _ in $(${pkgs.coreutils}/bin/seq 1 100); do
      test ! -e ${lib.escapeShellArg agentPrivateSocket} && break
      sleep 0.05
    done
    test ! -e ${lib.escapeShellArg agentPrivateSocket}
    ${python} ${launcher} \
      --server \
      --socket ${lib.escapeShellArg agentPrivateSocket} \
      --settings-root ${lib.escapeShellArg "${package}/share/gas-city-contributor/copilot"} \
      --copilot ${lib.escapeShellArg copilot} \
      --state-root ${lib.escapeShellArg "${stateRoot}/agent-state"} \
      --worktree ${lib.escapeShellArg "${stateRoot}/worktrees"} \
      --lease-root ${lib.escapeShellArg "${stateRoot}/leases"} \
      --runtime-root ${lib.escapeShellArg "${runtimeRoot}/agent"} \
      --runtime-path ${lib.escapeShellArg "${package}/bin"} \
      --runtime-path ${lib.escapeShellArg "${package}/share/gas-city-contributor"} \
      --sandbox-script ${lib.escapeShellArg sandbox} \
      --fdproxy-script ${lib.escapeShellArg fdproxy} \
      --sandbox-python ${lib.escapeShellArg python} \
      --bwrap-path ${lib.escapeShellArg bwrap} \
      --max-agents ${toString cfg.resources.maxConcurrentAgents} \
      --max-active-runs ${toString cfg.resources.maxActiveRuns} \
      --client-uid ${toString config.users.users.gascity-agent.uid} \
      --generation ${lib.escapeShellArg generation} \
      --state-schema 1 \
      --require-ready \
      --readiness-status ${lib.escapeShellArg readinessPath} &
    launcher_pid="$!"
    for _ in $(${pkgs.coreutils}/bin/seq 1 100); do
      test -S ${lib.escapeShellArg agentPrivateSocket} && break
      kill -0 "$launcher_pid" 2>/dev/null || exit 1
      sleep 0.05
    done
    test -S ${lib.escapeShellArg agentPrivateSocket}
    env -u COPILOT_GITHUB_TOKEN ${python} ${activation} agent-relay \
      --public-socket ${lib.escapeShellArg agentSocket} \
      --private-socket ${lib.escapeShellArg agentPrivateSocket} \
      --socket-group gascity-agent-channel \
      --allowed-uid ${toString config.users.users.gascity.uid} &
    relay_pid="$!"
    wait "$launcher_pid"
  '';

  discordStart = pkgs.writeShellScript "gascity-discord-start" ''
    set -euo pipefail
    test -s "$CREDENTIALS_DIRECTORY/discord-bot-token"
    exec ${python} ${activation} fdproxy-sidecar \
      --egress-socket ${lib.escapeShellArg egressSocket} \
      --fdproxy ${lib.escapeShellArg fdproxy} \
      --listen 127.0.0.1:3128 \
      -- \
      ${python} ${discordDecision} serve \
        --socket ${lib.escapeShellArg discordSocket} \
        --socket-group gascity-discord-channel \
        --credential "$CREDENTIALS_DIRECTORY/discord-bot-token" \
        --state-root ${lib.escapeShellArg discordStateRoot} \
        --guild-id ${lib.escapeShellArg cfg.discord.guildId} \
        --channel-id ${lib.escapeShellArg cfg.discord.channelId} \
        --operator-user-id ${lib.concatStringsSep " --operator-user-id "
          (map lib.escapeShellArg cfg.discord.operatorUserIds)} \
        --api-base https://discord.com/api/v10 \
        --gateway-url ${lib.escapeShellArg "wss://gateway.discord.gg/?v=10&encoding=json"}
  '';

  publisherStart = pkgs.writeShellScript "gascity-publisher-start" ''
    set -euo pipefail
    test -s "$CREDENTIALS_DIRECTORY/github-app-private-key"
    exec ${python} ${activation} fdproxy-sidecar \
      --egress-socket ${lib.escapeShellArg egressSocket} \
      --fdproxy ${lib.escapeShellArg fdproxy} \
      --listen 127.0.0.1:3128 \
      -- \
      ${python} ${publisher} serve \
        --socket ${lib.escapeShellArg publisherSocket} \
        --socket-group gascity-publisher-channel \
        --credential "$CREDENTIALS_DIRECTORY/github-app-private-key" \
        --state-root ${lib.escapeShellArg publisherStateRoot} \
        --repository ${lib.escapeShellArg cfg.repository.githubSlug} \
        --base-branch ${lib.escapeShellArg cfg.repository.baseBranch} \
        --branch-namespace gascity/ \
        --app-id ${lib.escapeShellArg cfg.github.appId} \
        --installation-id ${lib.escapeShellArg cfg.github.installationId} \
        --api-base https://api.github.com \
        --cancellation-root ${lib.escapeShellArg cancellationRoot}
  '';

  mainEnvironment = [
    "HOME=${serviceRoot}/home"
    "XDG_CONFIG_HOME=${serviceRoot}/home/.config"
    "XDG_STATE_HOME=${serviceRoot}/home/.local/state"
    "XDG_CACHE_HOME=${cacheRoot}"
    "XDG_RUNTIME_DIR=${runtimeRoot}"
    "GC_HOME=${serviceRoot}/gc"
    "GC_CONTRIBUTOR_ROOT=${package}/share/gas-city-contributor"
    "GC_RIG_NAME=${cfg.repository.rigName}"
    "GC_REPOSITORY=${cfg.repository.githubSlug}"
    "GC_BASE_BRANCH=${cfg.repository.baseBranch}"
    "GC_GITHUB_APP_ID=${cfg.github.appId}"
    "GC_GITHUB_INSTALLATION_ID=${cfg.github.installationId}"
    "GC_DISCORD_GUILD_ID=${cfg.discord.guildId}"
    "GC_DISCORD_CHANNEL_ID=${cfg.discord.channelId}"
    "GC_SUPERVISOR_SYSTEMD_UNIT=gas-city-contributor.service"
    "GC_SUPERVISOR_SYSTEMD_SCOPE=system"
    "GC_DISABLE_BINARY_DRIFT_RESTART=1"
    "GC_REQUIRE_READINESS=1"
    "GC_SUPERVISOR_BIND=127.0.0.1:${toString cfg.ports.supervisor}"
    "GC_SUPERVISOR_PORT=${toString cfg.ports.supervisor}"
    "GC_DOLT_BIND=127.0.0.1:${toString cfg.ports.dolt}"
    "GC_DOLT_PORT=${toString cfg.ports.dolt}"
    "GC_AGENT_LAUNCHER_SOCKET=${agentSocket}"
    "GC_DISCORD_CHANNEL_SOCKET=${discordSocket}"
    "GC_PUBLISHER_CHANNEL_SOCKET=${publisherSocket}"
    "GC_CANCEL_ROOT=${cancellationRoot}"
    "GC_EGRESS_SOCKET=${egressSocket}"
    "GC_FDPROXY_AUTH=${relayAuth}"
    "GC_PROJECT_QUOTA_REQUIRED=1"
    "GC_MANUAL_CLEANUP_ONLY=1"
  ] ++ lib.optionals cfg.check.enable [
    "GC_CHECK_SOCKET=${checkSocket}"
    "GC_CHECK_AUTH=${checkAuth}"
  ];

  mainExec = lib.concatStringsSep " " [
    "${package}/bin/gc"
    "supervisor"
    "run"
  ];

  sidecarUnit = {
    unitConfig = {
      PartOf = "gas-city-contributor.service";
      Before = "gas-city-contributor.service";
      StartLimitIntervalSec = 60;
      StartLimitBurst = 5;
    };
    serviceConfig = sharedServiceConfig;
  };
in
{
  config = lib.mkIf cfg.enable {
    systemd.slices.gascity-contributor = {
      description = "Gas City contributor aggregate resource boundary";
      sliceConfig = {
        CPUQuota = "${toString cfg.resources.cpuQuotaPercent}%";
        MemoryHigh = "${toString cfg.resources.memoryHighPercent}%";
        MemoryMax = "${toString cfg.resources.memoryMaxPercent}%";
        MemorySwapMax = toString cfg.resources.memorySwapMaxBytes;
        TasksMax = cfg.resources.tasksMax;
      };
    };

    systemd.services =
      {
        gas-city-contributor = {
          description = "Gas City contributor lifecycle supervisor";
          wantedBy = [ "multi-user.target" ];
          unitConfig = {
            StartLimitIntervalSec = 60;
            StartLimitBurst = 5;
          };
          requires = [
            "gascity-agent.service"
            "gascity-discord.service"
            "gascity-publisher.service"
            "gascity-egress.service"
            "gascity-free-space-monitor.service"
          ]
          ++ lib.optional cfg.check.enable "gascity-check.service"
          ++ lib.optional buildBuddyEnabled "gascity-buildbuddy-proxy.service";
          after = [
            "gascity-agent.service"
            "gascity-discord.service"
            "gascity-publisher.service"
            "gascity-egress.service"
          ]
          ++ lib.optionals cfg.check.enable [ "gascity-check.service" ]
          ++ lib.optionals buildBuddyEnabled [ "gascity-buildbuddy-proxy.service" ];
          serviceConfig = sharedServiceConfig // {
            Type = "exec";
            User = "gascity";
            Group = "gascity-contributor";
            SupplementaryGroups = [
              "gascity-contributor"
              "gascity-agent-channel"
              "gascity-discord-channel"
              "gascity-publisher-channel"
            ] ++ lib.optional cfg.check.enable "gascity-check-channel";
            WorkingDirectory = serviceRoot;
            StateDirectory = "gascity-contributor";
            StateDirectoryMode = "0710";
            StateDirectoryQuota = toString cfg.storage.stateQuotaBytes;
            CacheDirectory = "gascity-contributor";
            CacheDirectoryMode = "0700";
            CacheDirectoryQuota = toString cfg.storage.cacheQuotaBytes;
            ReadWritePaths = [
              serviceRoot
              stateRoot
              cacheRoot
              runtimeRoot
            ];
            ReadOnlyPaths = [ "${package}/share/gas-city-contributor" ];
            BindReadOnlyPaths = map (
              path:
              "${path}:/run/gascity-contributor/host/${builtins.baseNameOf path}"
            ) cfg.hostReadOnlyPaths;
            IPAddressDeny = [ "any" ];
            IPAddressAllow = [ "127.0.0.0/8" "::1/128" ];
            Environment = mainEnvironment;
            ExecStartPre = [
              validatePaths
              checkReserve
              materialize
              waitReadiness
              "${pkgs.coreutils}/bin/install -d -m 0700 ${lib.escapeShellArg "${serviceRoot}/home/.config"}"
              "${pkgs.coreutils}/bin/install -d -m 0700 ${lib.escapeShellArg "${serviceRoot}/home/.local/state"}"
              "${pkgs.coreutils}/bin/install -d -m 0700 ${lib.escapeShellArg "${stateRoot}/rigs/${cfg.repository.rigName}"}"
            ];
            ExecStart = mainExec;
            ExecStartPost = decisionReconcile;
            TimeoutStartSec = "5min";
            TimeoutStopSec = "2min";
          };
        };

        gascity-agent = {
          description = "Gas City ACP launcher";
          requires = [ "gascity-egress.service" ];
          after = [ "gascity-egress.service" ];
          inherit (sidecarUnit) unitConfig;
          serviceConfig = sharedServiceConfig // {
            Type = "exec";
            User = "gascity-agent";
            Group = "gascity-agent-channel";
            SupplementaryGroups = [ "gascity-contributor" "gascity-egress-channel" "gascity-agent-channel" ];
            PrivateNetwork = true;
            RuntimeDirectory = "gascity-agent";
            RuntimeDirectoryMode = "0700";
            StateDirectory = "gascity-agent";
            StateDirectoryMode = "0700";
            ReadWritePaths = [
              stateRoot
              runtimeRoot
            ];
            InaccessiblePaths = commonServiceConfig.InaccessiblePaths ++ [
              "-${discordSocket}"
              "-${publisherSocket}"
            ];
            ReadOnlyPaths = [ "${package}/share/gas-city-contributor" ];
            LoadCredential = [
              "copilot-token:${cfg.credentials.copilotTokenFile}"
            ];
            Environment = [
              "GC_FDPROXY_SOCKET=${egressSocket}"
              "GC_FDPROXY_AUTH=${relayAuth}"
              "GC_AGENT_LAUNCHER_SOCKET=${agentPrivateSocket}"
              "GC_REQUIRE_READINESS=1"
              "PATH=${package}/bin:/run/current-system/sw/bin"
              "SSL_CERT_FILE=${package}/etc/ssl/certs/ca-bundle.crt"
            ];
            ExecStart = agentStart;
          };
        };

        gascity-discord = {
          description = "Gas City Discord integration boundary";
          requires = [ "gascity-egress.service" ];
          after = [ "gascity-egress.service" ];
          inherit (sidecarUnit) unitConfig;
          serviceConfig = sharedServiceConfig // {
            Type = "exec";
            User = "gascity-discord";
            Group = "gascity-discord";
            SupplementaryGroups = [
              "gascity-contributor"
              "gascity-discord-channel"
              "gascity-egress-channel"
            ];
            PrivateNetwork = true;
            StateDirectory = "gascity-discord";
            StateDirectoryMode = "0700";
            StateDirectoryQuota = toString cfg.storage.discordQuotaBytes;
            LoadCredential = [
              "discord-bot-token:${cfg.credentials.discordBotTokenFile}"
            ];
            Environment =
              [
                "GC_DISCORD_APPLICATION_ID=${cfg.discord.applicationId}"
                "GC_DISCORD_GUILD_ID=${cfg.discord.guildId}"
                "GC_DISCORD_CHANNEL_ID=${cfg.discord.channelId}"
                "GC_EGRESS_SOCKET=${egressSocket}"
                "GC_FDPROXY_SOCKET=${egressSocket}"
                "GC_FDPROXY_AUTH=${relayAuth}"
                "SSL_CERT_FILE=${package}/etc/ssl/certs/ca-bundle.crt"
              ]
              ++ sidecarProxyEnvironment;
            ReadWritePaths = [ discordStateRoot runtimeRoot ];
            ReadOnlyPaths = [ "${package}/share/gas-city-contributor" ];
            ExecStart = discordStart;
          };
        };

        gascity-publisher = {
          description = "Gas City GitHub publication boundary";
          requires = [ "gascity-egress.service" ];
          after = [ "gascity-egress.service" ];
          inherit (sidecarUnit) unitConfig;
          serviceConfig = sharedServiceConfig // {
            Type = "exec";
            User = "gascity-publisher";
            Group = "gascity-publisher";
            SupplementaryGroups = [
              "gascity-contributor"
              "gascity-publisher-channel"
              "gascity-egress-channel"
            ];
            PrivateNetwork = true;
            StateDirectory = "gascity-publisher";
            StateDirectoryMode = "0700";
            StateDirectoryQuota = toString cfg.storage.publisherQuotaBytes;
            LoadCredential = [
              "github-app-private-key:${cfg.credentials.githubPrivateKeyFile}"
            ];
            Environment = [
              "GC_GITHUB_APP_ID=${cfg.github.appId}"
              "GC_GITHUB_INSTALLATION_ID=${cfg.github.installationId}"
              "GC_REPOSITORY=${cfg.repository.githubSlug}"
              "GC_PUBLISHER_CHANNEL_SOCKET=${publisherSocket}"
              "GC_EGRESS_SOCKET=${egressSocket}"
              "GC_FDPROXY_SOCKET=${egressSocket}"
              "GC_FDPROXY_AUTH=${relayAuth}"
              "SSL_CERT_FILE=${package}/etc/ssl/certs/ca-bundle.crt"
            ]
            ++ sidecarProxyEnvironment;
            ReadWritePaths = [ publisherStateRoot cancellationRoot runtimeRoot ];
            ReadOnlyPaths = [
              "${package}/share/gas-city-contributor"
            ];
            ExecStart = publisherStart;
          };
        };

        gascity-free-space-monitor = {
          description = "Gas City contributor free-space reserve monitor";
          requires = [ "gas-city-contributor.service" ];
          after = [ "gas-city-contributor.service" ];
          unitConfig = {
            PartOf = "gas-city-contributor.service";
            StartLimitIntervalSec = 60;
            StartLimitBurst = 5;
          };
          serviceConfig = sharedServiceConfig // {
            Type = "exec";
            User = "gascity";
            Group = "gascity";
            SupplementaryGroups = [ "gascity-contributor" ];
            ExecStart = "${python} ${activation} free-space-monitor"
              + " --path ${lib.escapeShellArg serviceRoot}"
              + " --reserve-bytes ${toString cfg.storage.minFreeBytes}"
              + " --interval 30";
          };
        };
      }
      // lib.optionalAttrs cfg.check.enable {
        gascity-check = {
          description = "Gas City uncredentialed local Nix check runner";
          requires =
            [ "gascity-egress.service" ]
            ++ lib.optional buildBuddyEnabled "gascity-buildbuddy-proxy.service";
          after =
            [ "gascity-egress.service" ]
            ++ lib.optional buildBuddyEnabled "gascity-buildbuddy-proxy.service";
          inherit (sidecarUnit) unitConfig;
          serviceConfig = sharedServiceConfig // {
            Type = "exec";
            User = "gascity-check";
            Group = "gascity-check-channel";
            SupplementaryGroups = [
              "gascity-contributor"
              "gascity-check"
              "gascity-check-channel"
              "gascity-egress-channel"
            ];
            PrivateNetwork = true;
            JoinsNamespaceOf = lib.optional buildBuddyEnabled "gascity-buildbuddy-proxy.service";
            StateDirectory = "gascity-check";
            StateDirectoryMode = "0700";
            StateDirectoryQuota = toString cfg.storage.checkQuotaBytes;
            ReadWritePaths = [
              "/var/lib/gascity-check"
              runtimeRoot
            ];
            ReadOnlyPaths = [
              "${package}/share/gas-city-contributor"
              "${stateRoot}/worktrees"
            ];
            InaccessiblePaths = [ "-/nix/var/nix/daemon-socket/socket" ];
            Environment = [
              "GC_CHECK_PROXY=http://127.0.0.1:3128"
              "GC_CHECK_OUTPUT_ROOT=/var/lib/gascity-check/output"
              "GC_CHECK_STORE_ROOT=/var/lib/gascity-check/nix-root"
              "NIX_REMOTE=local?root=/var/lib/gascity-check/nix-root"
              "GC_FDPROXY_SOCKET=${egressSocket}"
              "GC_FDPROXY_AUTH=${relayAuth}"
              "GC_CHECK_AUTH=${checkAuth}"
              "NIX_CONFIG=connect-timeout = 5\nmax-jobs = ${toString cfg.resources.nixMaxJobs}\ncores = ${toString cfg.resources.nixBuildCores}\nhttp-proxy = http://127.0.0.1:3128\nsubstituters = https://cache.nixos.org\ntrusted-public-keys = cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY="
            ];
            ExecStart = "${python} ${package}/share/gas-city-contributor/pack/scripts/check-runner.py"
              + " server"
              + " --store-root /var/lib/gascity-check/nix-root"
              + " --output-root /var/lib/gascity-check/output"
              + " --proxy http://127.0.0.1:3128"
              + " --egress-socket ${lib.escapeShellArg egressSocket}"
              + " --socket ${lib.escapeShellArg checkSocket}"
              + " --allowed-uid ${toString config.users.users.gascity.uid}"
              + " --check-auth-token-env GC_CHECK_AUTH"
              + " --approved-check ${lib.escapeShellArg
                "build-artifact-valid=.gc/scripts/checks/build-artifact-valid.sh"}"
              + " --max-jobs ${toString cfg.resources.nixMaxJobs}"
              + " --build-cores ${toString cfg.resources.nixBuildCores}"
              + " --max-heavy-checks ${toString cfg.resources.maxHeavyChecks}"
              + " --timeout-seconds ${toString cfg.resources.checkTimeoutSeconds}"
              + " --term-grace 2"
              + " --kill-grace 1";
            TimeoutStopSec = "2min";
          };
        };
      }
      // lib.optionalAttrs buildBuddyEnabled {
        gascity-buildbuddy-proxy = {
          description = "Gas City BuildBuddy credential proxy";
          requires = [ "gascity-egress.service" ];
          after = [ "gascity-egress.service" ];
          inherit (sidecarUnit) unitConfig;
          serviceConfig = sharedServiceConfig // {
            Type = "exec";
            User = "gascity-buildbuddy-proxy";
            Group = "gascity-buildbuddy-proxy";
            SupplementaryGroups = [ "gascity-contributor" "gascity-egress-channel" ];
            PrivateNetwork = true;
            LoadCredential = [
              "buildbuddy-api-key:${cfg.credentials.buildBuddyApiKeyFile}"
            ];
            ReadOnlyPaths = [
              "${package}/share/gas-city-contributor"
              "${package}/etc/ssl/certs/ca-bundle.crt"
            ];
            ReadWritePaths = [ runtimeRoot ];
            Environment = [
              "SSL_CERT_FILE=${package}/etc/ssl/certs/ca-bundle.crt"
              "GC_BUILDBUDDY_UPSTREAM=remote.buildbuddy.io:443"
              "GC_FDPROXY_SOCKET=${egressSocket}"
              "GC_FDPROXY_AUTH=${relayAuth}"
            ];
            ExecStart = "${python} ${package}/share/gas-city-contributor/pack/scripts/buildbuddy-proxy.py"
              + " serve"
              + " --template ${lib.escapeShellArg "${package}/share/gas-city-contributor/buildbuddy/envoy.yaml.tmpl"}"
              + " --credential %d/buildbuddy-api-key"
              + " --envoy ${lib.escapeShellArg "${package}/bin/envoy"}"
              + " --listen 127.0.0.1:19801"
              + " --egress-socket ${lib.escapeShellArg egressSocket}"
              + " --ca ${lib.escapeShellArg "${package}/etc/ssl/certs/ca-bundle.crt"}";
          };
        };
      };
  };
}
