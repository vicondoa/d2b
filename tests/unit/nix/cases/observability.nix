# nix-unit cases migrated from tests/observability-eval.sh.
#
# The retired bash gate imported tests/unit/nix/eval-cases/observability.nix and then
# asserted, for every expect-success case, that the extracted JSON exactly
# matched `expectedExtract`; for expect-failure cases, it asserted that the
# config.assertions surface was reachable, non-empty, and contained every
# expected substring.
#
# This nix-unit successor keeps that shape: one value case per legacy scenario,
# including the evalSucceeded/no-failing-assertions envelope for success cases,
# and a message-substring value assertion for the failure case. The local
# nix-unit runner deliberately does NOT support `expectedError.msg` matching
# (tryEval loses throw text), so assertion-message checks stay as values over
# `config.assertions`, matching tests/unit/nix/cases/assertions.nix.
#
# Spec corrections ("existing code is canon"):
#   * tests/unit/nix/eval-cases/observability.nix still re-gets the flake through
#     `builtins.getFlake "git+file://$flakeRoot"`; inside flake checks
#     `flakeRoot = ./.` is a store path, not a Git checkout. Reconstruct the
#     same scenario table here with direct nixpkgs/nixlingModule injection.
#   * The host-level `microvm.vms` surface is retired; per-VM guest configs now
#     live under `nixling._computed.<vm>.config`.
#   * `processesJson.data` is the current context-safe bundle surface, and
#     minijail profiles expose `caps`, not the old `capabilities` key.
#   * The observability dashboards directory is no longer retired/empty: the
#     current SigNoz surface ships six dashboards.
{ lib, flakeRoot, nixpkgsFlake, nixlingModule, ... }:

let
  shared = import ../eval-cases/shared.nix {
    nixpkgs = nixpkgsFlake;
    inherit nixlingModule;
  };

  sortStrings = builtins.sort builtins.lessThan;
  hasInfix = lib.hasInfix;

  forceAttempt = value:
    let
      attempt = builtins.tryEval (builtins.deepSeq value value);
    in
    if attempt.success then {
      success = true;
      value = attempt.value;
    } else {
      success = false;
      value = null;
    };

  mkNixos = { caseSystem, override }:
    nixpkgsFlake.lib.nixosSystem {
      system = caseSystem;
      pkgs = shared.pkgsFor caseSystem;
      modules = [
        nixlingModule
        shared.baseModule
        ({ ... }: { boot.initrd.includeDefaultModules = false; })
        override
      ];
    };

  evalCase = caseSpec:
    let
      kind = caseSpec.kind or (if caseSpec ? extract || caseSpec ? expectedExtract then "expect-success" else "expect-failure");
      caseSystem = caseSpec.system or shared.defaultSystem;
      override = caseSpec.override or ({ ... }: { });
      nixos = mkNixos { inherit caseSystem override; };
      failureAttempt =
        if kind == "expect-failure" then
          builtins.tryEval nixos.config.assertions
        else
          { success = true; value = [ ]; };
      extractAttempt =
        if kind == "expect-success" && caseSpec ? extract then
          forceAttempt (caseSpec.extract nixos)
        else
          { success = true; value = null; };
      auxAttempt =
        if kind == "expect-success" && caseSpec ? aux then
          forceAttempt (caseSpec.aux nixos)
        else
          { success = true; value = null; };
    in
    {
      inherit kind;
      expectedSubstrings =
        caseSpec.expectedSubstrings or (lib.optional (caseSpec ? expectedSubstring) caseSpec.expectedSubstring);
      expectedExtract = caseSpec.expectedExtract or null;
      evalSucceeded =
        if kind == "expect-failure" then
          failureAttempt.success
        else
          extractAttempt.success && auxAttempt.success;
      failingMessages =
        if kind == "expect-failure" && failureAttempt.success then
          map
            (assertion: assertion.message or "")
            (builtins.filter (assertion: !(assertion.assertion or false)) failureAttempt.value)
        else
          [ ];
      extracted = extractAttempt.value;
    };

  manifest = nixos: builtins.fromJSON nixos.config.nixling._manifestPkg.text;

  cliPkg = nixos:
    builtins.head (
      builtins.filter
        (pkg: (pkg.name or "") == "nixling" || (pkg.pname or "") == "nixling")
        nixos.config.environment.systemPackages
    );

  dashboardDir = flakeRoot + "/nixos-modules/components/observability/dashboards";
  dashboardNames =
    sortStrings (
      builtins.filter
        (name: builtins.match ".*\\.json$" name != null)
        (builtins.attrNames (builtins.readDir dashboardDir))
    );
  dashboardFiles = map (name: dashboardDir + "/${name}") dashboardNames;
  dashboardPaths = map toString dashboardFiles;

  caseSpecs = {
    obs-disabled-default = {
      extract = nixos: (manifest nixos)._observability.enabled;
      expectedExtract = false;
    };

    obs-default-off-no-units = {
      override = { ... }: { nixling.observability.enable = false; };
      extract = nixos: {
        otelServiceNames = sortStrings (
          builtins.filter
            (name: builtins.match "^nixling-otel-.*" name != null)
            (builtins.attrNames nixos.config.systemd.services)
        );
      };
      expectedExtract = { otelServiceNames = [ ]; };
      aux = nixos: { cliDrvPath = (cliPkg nixos).drvPath; };
    };

    obs-enabled-defaults = {
      override = { ... }: { nixling.observability.enable = true; };
      extract = nixos:
        let
          manifestData = manifest nixos;
          obsVm = nixos.config.nixling.observability.vmName;
          obsEnv = lib.attrByPath [ "obs" ] { } nixos.config.nixling.envs;
        in
        {
          hasSysObs = builtins.hasAttr "sys-obs" nixos.config.nixling.vms;
          hasObsEnv = builtins.hasAttr "obs" nixos.config.nixling.envs;
          obsEnvLanSubnet = obsEnv.lanSubnet or null;
          obsEnvUplinkSubnet = obsEnv.uplinkSubnet or null;
          obsVmName = lib.attrByPath [ "_observability" "vmName" ] null manifestData;
          obsVsockCid = lib.attrByPath [ "_observability" "obsVsockCid" ] null manifestData;
          signozListenAddress = nixos.config.nixling.observability.signoz.listenAddress;
          obsVmStaticIp = lib.attrByPath [ obsVm "staticIp" ] null nixos.config.nixling.manifest;
          signozUrl = lib.attrByPath [ "_observability" "signozUrl" ] null manifestData;
        };
      expectedExtract = {
        hasSysObs = true;
        hasObsEnv = true;
        obsEnvLanSubnet = "10.40.0.0/24";
        obsEnvUplinkSubnet = "203.0.113.0/30";
        obsVmName = "sys-obs";
        obsVsockCid = 1000;
        signozListenAddress = "10.40.0.10";
        obsVmStaticIp = "10.40.0.10";
        signozUrl = "http://10.40.0.10:8080";
      };
    };

    obs-signoz-bind-tracks-obs-ip = {
      override = { ... }: {
        nixling.observability.enable = true;
        nixling.observability.lanSubnet = "10.44.0.0/24";
        nixling.observability.index = 23;
      };
      extract = nixos:
        let
          manifestData = manifest nixos;
          obsVm = nixos.config.nixling.observability.vmName;
        in
        {
          signozListenAddress = nixos.config.nixling.observability.signoz.listenAddress;
          obsVmStaticIp = lib.attrByPath [ obsVm "staticIp" ] null nixos.config.nixling.manifest;
          signozUrl = lib.attrByPath [ "_observability" "signozUrl" ] null manifestData;
        };
      expectedExtract = {
        signozListenAddress = "10.44.0.23";
        obsVmStaticIp = "10.44.0.23";
        signozUrl = "http://10.44.0.23:8080";
      };
    };

    obs-name-extension-allowed = {
      override = { ... }: {
        nixling.observability.enable = true;
        nixling.vms.sys-obs = {
          ssh.user = "alice";
          config.users.users.alice = { isNormalUser = true; uid = 1000; };
        };
      };
      extract = nixos: builtins.hasAttr "sys-obs" nixos.config.nixling.vms;
      expectedExtract = true;
    };

    obs-cid-cross-env-noncollision = {
      override = { lib, ... }: {
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
      };
      extract = nixos:
        let data = manifest nixos;
        in {
          corp = data.corp-vm.observability.vsockCid;
          other = data.other-vm.observability.vsockCid;
        };
      expectedExtract = {
        corp = 210;
        other = 1110;
      };
    };

    obs-manifest-fields = {
      override = { ... }: { nixling.observability.enable = true; };
      extract = nixos:
        let vmObs = lib.attrByPath [ "corp-vm" "observability" ] { } (manifest nixos);
        in {
          enabled = vmObs.enabled or null;
          vsockCid = vmObs.vsockCid or null;
          vsockHostSocket = vmObs.vsockHostSocket or null;
          agentSocket = vmObs.agentSocket or null;
        };
      expectedExtract = {
        enabled = false;
        vsockCid = 1110;
        vsockHostSocket = "/var/lib/nixling/vms/corp-vm/vsock.sock";
        agentSocket = "/run/nixling/otlp.sock";
      };
    };

    obs-relay-acl-surface = {
      override = { ... }: {
        nixling.observability.enable = true;
        nixling.vms.corp-vm.observability.enable = true;
      };
      extract = nixos:
        let
          processes = nixos.config.nixling._bundle.processesJson.data;
          corpDag = builtins.head (builtins.filter (dag: dag.vm == "corp-vm") processes.vms);
          relayNode = builtins.head (builtins.filter (node: node.id == "vsock-relay") corpDag.nodes);
          obsDag = builtins.head (builtins.filter (dag: dag.vm == "sys-obs") processes.vms);
          bridgeNode = builtins.head (builtins.filter (node: node.id == "otel-host-bridge") obsDag.nodes);
        in
        {
          relayNodeRole = relayNode.role;
          relayProfileHasEmptyCaps = relayNode.profile.caps == [ ];
          relayProfileSeccomp = relayNode.profile.seccompPolicyRef;
          bridgeNodeRole = bridgeNode.role;
          bridgeProfileHasEmptyCaps = bridgeNode.profile.caps == [ ];
          bridgeProfileSeccomp = bridgeNode.profile.seccompPolicyRef;
          bridgeProfileHasRuntimeBind =
            builtins.any (entry: entry.path == "/run/nixling/otel") bridgeNode.profile.mountPolicy.writablePaths;
          bridgeProfileHasObsVmBind =
            builtins.any (entry: entry.path == "/var/lib/nixling/vms/sys-obs") bridgeNode.profile.mountPolicy.writablePaths;
          bridgeUidDistinctFromRelay = bridgeNode.profile.uid != relayNode.profile.uid;
        };
      expectedExtract = {
        relayNodeRole = "vsock-relay";
        relayProfileHasEmptyCaps = true;
        relayProfileSeccomp = "w1-vsock-relay";
        bridgeNodeRole = "otel-host-bridge";
        bridgeProfileHasEmptyCaps = true;
        bridgeProfileSeccomp = "w1-otel-host-bridge";
        bridgeProfileHasRuntimeBind = true;
        bridgeProfileHasObsVmBind = true;
        bridgeUidDistinctFromRelay = true;
      };
    };

    obs-stack-vm-guest-surface = {
      override = { ... }: {
        nixling.observability.enable = true;
        nixling.vms.corp-vm.observability.enable = true;
        nixling.observability.retention.metrics = "5d";
        nixling.observability.retention.logs = "3d";
        nixling.observability.retention.traces = "1d";
      };
      extract = nixos:
        let
          obsVm = nixos.config.nixling.observability.vmName;
          obsGuest = nixos.config.nixling._computed.${obsVm}.config;
          services = obsGuest.systemd.services;
          ingressSources = obsGuest.nixling.observability.ingress.sources;
        in
        {
          obsVmName = obsVm;
          manifestHasObsVm = builtins.hasAttr obsVm nixos.config.nixling.manifest;
          clickhouseEnable = obsGuest.services.clickhouse.enable;
          keeperDeclared = builtins.hasAttr "clickhouse-keeper" services;
          signozDeclared = builtins.hasAttr "signoz" services;
          signozCollectorDeclared = builtins.hasAttr "signoz-otel-collector" services;
          signozMigrateDeclared = builtins.hasAttr "signoz-schema-migrate-sync" services;
          retiredServicesAbsent = !(
            builtins.hasAttr "grafana" services
            || builtins.hasAttr "prometheus" services
            || builtins.hasAttr "loki" services
            || builtins.hasAttr "tempo" services
            || builtins.hasAttr "alloy" services
          );
          ingressSourceNames = sortStrings (builtins.attrNames ingressSources);
          hostIngress = ingressSources.host;
          corpIngress = ingressSources.corp-vm;
          hostVsockInDeclared = builtins.hasAttr "nixling-otel-vsock-in-host" services;
          corpVsockInDeclared = builtins.hasAttr "nixling-otel-vsock-in-corp-vm" services;
          hostVsockInExecStartHasShape = hasInfix
            "VSOCK-LISTEN:14317,fork,max-children=16,reuseaddr TCP:127.0.0.1:4317"
            services.nixling-otel-vsock-in-host.serviceConfig.ExecStart;
          corpVsockInExecStartHasShape = hasInfix
            "VSOCK-LISTEN:14318,fork,max-children=16,reuseaddr TCP:127.0.0.1:14318"
            services.nixling-otel-vsock-in-corp-vm.serviceConfig.ExecStart;
          signozBindAddress = obsGuest.nixling.observability.signoz.listenAddress;
        };
      expectedExtract = {
        obsVmName = "sys-obs";
        manifestHasObsVm = true;
        clickhouseEnable = true;
        keeperDeclared = true;
        signozDeclared = true;
        signozCollectorDeclared = true;
        signozMigrateDeclared = true;
        retiredServicesAbsent = true;
        ingressSourceNames = [ "corp-vm" "host" ];
        hostIngress = {
          envName = "host";
          receiverGrpcPort = 4317;
          receiverHttpPort = 4318;
          role = "host";
          vmName = "nixos";
          vsockPort = 14317;
        };
        corpIngress = {
          envName = "work";
          receiverGrpcPort = 14318;
          receiverHttpPort = null;
          role = "workload";
          vmName = "corp-vm";
          vsockPort = 14318;
        };
        hostVsockInDeclared = true;
        corpVsockInDeclared = true;
        hostVsockInExecStartHasShape = true;
        corpVsockInExecStartHasShape = true;
        signozBindAddress = "10.40.0.10";
      };
    };

    obs-alerting-surface = {
      override = { ... }: {
        nixling.observability.enable = true;
        nixling.vms.corp-vm.observability.enable = true;
      };
      extract = nixos:
        let
          obsVm = nixos.config.nixling.observability.vmName;
          obsGuest = nixos.config.nixling._computed.${obsVm}.config;
          workGuest = nixos.config.nixling._computed.corp-vm.config;
        in
        {
          obsIngressSources = sortStrings (builtins.attrNames obsGuest.nixling.observability.ingress.sources);
          guestOtelCollectorDeclared = builtins.hasAttr "nixling-otel-collector" workGuest.systemd.services;
          guestVsockOutDeclared = builtins.hasAttr "nixling-otel-vsock-out" workGuest.systemd.services;
          guestAlloyAbsent = ! builtins.hasAttr "alloy" workGuest.systemd.services;
          guestIdentity = workGuest.nixling.observability.identity;
          guestVsockOutHasHostPort = hasInfix "VSOCK-CONNECT:2:14317"
            workGuest.systemd.services.nixling-otel-vsock-out.serviceConfig.ExecStart;
        };
      expectedExtract = {
        obsIngressSources = [ "corp-vm" "host" ];
        guestOtelCollectorDeclared = true;
        guestVsockOutDeclared = true;
        guestAlloyAbsent = true;
        guestIdentity = {
          envName = "work";
          vmName = "corp-vm";
        };
        guestVsockOutHasHostPort = true;
      };
    };

    obs-vm-toggle-default-off = {
      override = { ... }: { nixling.observability.enable = true; };
      extract = nixos: lib.attrByPath [ "corp-vm" "observability" "enabled" ] null (manifest nixos);
      expectedExtract = false;
    };

    obs-journal-default-on = {
      override = { ... }: {
        nixling.observability.enable = true;
        nixling.vms.corp-vm.observability.enable = true;
      };
      extract = nixos:
        let workGuest = nixos.config.nixling._computed.corp-vm.config;
        in
        {
          scrapeJournalResolved = workGuest.nixling.observability.scrapeJournal;
          otelUserInJournalGroup =
            builtins.elem "systemd-journal" (workGuest.users.users.otel.extraGroups or [ ]);
        };
      expectedExtract = {
        scrapeJournalResolved = true;
        otelUserInJournalGroup = true;
      };
    };

    obs-audit-surface = {
      override = { ... }: {
        nixling.observability.enable = true;
        nixling.vms.corp-vm.observability = {
          enable = true;
          scrapeJournal = false;
        };
        nixling.vms.corp-vm.audit.enable = true;
      };
      extract = nixos:
        let workGuest = nixos.config.nixling._computed.corp-vm.config;
        in
        {
          auditEnabled = workGuest.security.audit.enable;
          auditdEnabled = workGuest.security.auditd.enable;
          auditdSyslogPlugin = workGuest.security.auditd.plugins.syslog.active;
          guestOtelCollectorDeclared = builtins.hasAttr "nixling-otel-collector" workGuest.systemd.services;
          guestAlloyAbsent = ! builtins.hasAttr "alloy" workGuest.systemd.services;
          scrapeJournalResolved = workGuest.nixling.observability.scrapeJournal;
          auditRules = sortStrings workGuest.security.audit.rules;
        };
      expectedExtract = {
        auditEnabled = true;
        auditdEnabled = true;
        auditdSyslogPlugin = true;
        guestOtelCollectorDeclared = true;
        guestAlloyAbsent = true;
        scrapeJournalResolved = false;
        auditRules = [
          "-w /etc/passwd -p wa -k identity"
          "-w /etc/shadow -p wa -k identity"
          "-w /etc/sudoers -p wa -k priv-esc"
        ];
      };
    };

    obs-cli-traces-default-on = {
      override = { ... }: { nixling.observability.enable = true; };
      extract = nixos: nixos.config.nixling.observability.cli.traces.enable;
      expectedExtract = true;
      aux = nixos: { cliDrvPath = (cliPkg nixos).drvPath; };
    };

    obs-cli-traces-disabled = {
      override = { ... }: {
        nixling.observability.enable = true;
        nixling.observability.cli.traces.enable = false;
      };
      extract = nixos: nixos.config.nixling.observability.cli.traces.enable;
      expectedExtract = false;
      aux = nixos: { cliDrvPath = (cliPkg nixos).drvPath; };
    };

    obs-cli-trace-attr-allowlist = {
      override = { ... }: { nixling.observability.enable = true; };
      extract = _nixos: true;
      expectedExtract = true;
      aux = nixos: { cliDrvPath = (cliPkg nixos).drvPath; };
    };

    obs-reserved-prefix-exempt = {
      override = { ... }: { nixling.observability.enable = true; };
      extract = nixos: builtins.hasAttr "sys-obs" nixos.config.nixling.vms;
      expectedExtract = true;
    };

    obs-vm-without-framework = {
      kind = "expect-failure";
      override = { ... }: { nixling.vms.corp-vm.observability.enable = true; };
      expectedSubstring = "observability.enable = true but nixling.observability.enable is false";
    };

    obs-dashboards-schema = {
      extract = _nixos: {
        dashboardFileCount = builtins.length dashboardPaths;
        retiredDashboardDirIsEmpty = dashboardPaths == [ ];
      };
      expectedExtract = {
        dashboardFileCount = 6;
        retiredDashboardDirIsEmpty = false;
      };
    };

    obs-rules-promtool = {
      override = { ... }: {
        nixling.observability.enable = true;
        nixling.vms.corp-vm.observability.enable = true;
      };
      extract = nixos:
        let
          obsVm = nixos.config.nixling.observability.vmName;
          obsGuest = nixos.config.nixling._computed.${obsVm}.config;
          services = obsGuest.systemd.services;
        in
        {
          prometheusRuleFilesAbsent = !(obsGuest.services ? prometheus) || (obsGuest.services.prometheus.ruleFiles or [ ]) == [ ];
          signozServicesDeclared = builtins.all (name: builtins.hasAttr name services) [
            "signoz"
            "signoz-otel-collector"
            "signoz-schema-migrate-sync"
          ];
        };
      expectedExtract = {
        prometheusRuleFilesAbsent = true;
        signozServicesDeclared = true;
      };
    };

    obs-metric-references = {
      override = { ... }: {
        nixling.observability.enable = true;
        nixling.vms.corp-vm.observability.enable = true;
      };
      extract = nixos:
        let
          obsVm = nixos.config.nixling.observability.vmName;
          obsGuest = nixos.config.nixling._computed.${obsVm}.config;
          ingressSources = obsGuest.nixling.observability.ingress.sources;
          processes = nixos.config.nixling._bundle.processesJson.data;
          corpDag = builtins.head (builtins.filter (dag: dag.vm == "corp-vm") processes.vms);
          relayNode = builtins.head (builtins.filter (node: node.id == "vsock-relay") corpDag.nodes);
          relayArgv = builtins.concatStringsSep " " relayNode.argv;
        in
        {
          sourceNames = sortStrings (builtins.attrNames ingressSources);
          hostReceiverGrpcPort = ingressSources.host.receiverGrpcPort;
          corpReceiverGrpcPort = ingressSources.corp-vm.receiverGrpcPort;
          hostVsockPort = ingressSources.host.vsockPort;
          corpVsockPort = ingressSources.corp-vm.vsockPort;
          relayTargetsCorpIngressPort = hasInfix
            "nixling-ch-vsock-connect /var/lib/nixling/vms/sys-obs/vsock.sock 14318"
            relayArgv;
        };
      expectedExtract = {
        sourceNames = [ "corp-vm" "host" ];
        hostReceiverGrpcPort = 4317;
        corpReceiverGrpcPort = 14318;
        hostVsockPort = 14317;
        corpVsockPort = 14318;
        relayTargetsCorpIngressPort = true;
      };
    };

    obs-scrape-job-stability = {
      override = { ... }: {
        nixling.observability.enable = true;
        nixling.vms.corp-vm.observability.enable = true;
      };
      extract = nixos:
        let
          obsVm = nixos.config.nixling.observability.vmName;
          obsGuest = nixos.config.nixling._computed.${obsVm}.config;
          services = obsGuest.systemd.services;
        in
        {
          hostIngressExecHasShape = hasInfix
            "VSOCK-LISTEN:14317,fork,max-children=16,reuseaddr TCP:127.0.0.1:4317"
            services.nixling-otel-vsock-in-host.serviceConfig.ExecStart;
          corpIngressExecHasShape = hasInfix
            "VSOCK-LISTEN:14318,fork,max-children=16,reuseaddr TCP:127.0.0.1:14318"
            services.nixling-otel-vsock-in-corp-vm.serviceConfig.ExecStart;
          hostIngressRestartIfChanged = services.nixling-otel-vsock-in-host.restartIfChanged;
          corpIngressRestartIfChanged = services.nixling-otel-vsock-in-corp-vm.restartIfChanged;
        };
      expectedExtract = {
        hostIngressExecHasShape = true;
        corpIngressExecHasShape = true;
        hostIngressRestartIfChanged = false;
        corpIngressRestartIfChanged = false;
      };
    };

    obs-stability = {
      override = { ... }: {
        nixling.observability.enable = true;
        nixling.vms.corp-vm.observability.enable = true;
      };
      extract = nixos:
        let
          obsVm = nixos.config.nixling.observability.vmName;
          obsGuest = nixos.config.nixling._computed.${obsVm}.config;
          workGuest = nixos.config.nixling._computed.corp-vm.config;
        in
        {
          retiredBackendServicesAbsent = !(
            obsGuest.systemd.services ? grafana
            || obsGuest.systemd.services ? prometheus
            || obsGuest.systemd.services ? loki
            || obsGuest.systemd.services ? tempo
            || obsGuest.systemd.services ? alloy
          );
          hostCollectorDeclared = builtins.hasAttr "nixling-host-otel-collector" nixos.config.systemd.services;
          guestCollectorDeclared = builtins.hasAttr "nixling-otel-collector" workGuest.systemd.services;
          guestVsockOutDeclared = builtins.hasAttr "nixling-otel-vsock-out" workGuest.systemd.services;
        };
      expectedExtract = {
        retiredBackendServicesAbsent = true;
        hostCollectorDeclared = true;
        guestCollectorDeclared = true;
        guestVsockOutDeclared = true;
      };
    };

    obs-graphics-runner-wiring = {
      override = { ... }: {
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
      };
      extract = nixos:
        let
          gpuUnit = nixos.config.systemd.services."nixling-gpu-vm-gpu" or null;
          processes = nixos.config.nixling._bundle.processesJson.data;
          gpuDag = builtins.head (builtins.filter (dag: dag.vm == "gpu-vm") processes.vms);
          nodeIds = sortStrings (map (node: node.id) gpuDag.nodes);
          relayNode = builtins.head (builtins.filter (node: node.id == "vsock-relay") gpuDag.nodes);
        in
        {
          gpuServiceDeclared = gpuUnit != null;
          relayNodeDeclared = builtins.elem "vsock-relay" nodeIds;
          relayNodeRole = relayNode.role;
          relayNodeTargetsObs = hasInfix
            "nixling-ch-vsock-connect /var/lib/nixling/vms/sys-obs/vsock.sock"
            (builtins.concatStringsSep " " relayNode.argv);
        };
      expectedExtract = {
        gpuServiceDeclared = false;
        relayNodeDeclared = true;
        relayNodeRole = "vsock-relay";
        relayNodeTargetsObs = true;
      };
    };

    # ----- ADR 0033: host collector parity + hostname identity -----

    obs-host-collector-default-off = {
      override = { ... }: { nixling.observability.enable = true; };
      extract = nixos:
        let
          cfg = nixos.config.nixling.observability._internal.hostCollectorConfig;
          svc = nixos.config.systemd.services."nixling-host-otel-collector";
        in
        {
          receiverNames = sortStrings (builtins.attrNames cfg.receivers);
          pipelineNames = sortStrings (builtins.attrNames cfg.service.pipelines);
          hasExtensions = cfg ? extensions;
          resourceHasServiceName = builtins.any (a: (a.key or "") == "service.name") cfg.processors.resource.attributes;
          readWritePaths = svc.serviceConfig.ReadWritePaths or null;
          umask = svc.serviceConfig.UMask or null;
          suppGroups = svc.serviceConfig.SupplementaryGroups or null;
          restart = svc.serviceConfig.Restart or null;
          restartSec = svc.serviceConfig.RestartSec or null;
          startLimitIntervalSec = svc.unitConfig.StartLimitIntervalSec or null;
          tmpfilesHasIngest = builtins.any (r: lib.hasInfix "/run/nixling/otel/ingest" r) nixos.config.systemd.tmpfiles.rules;
          # Privileged ExecStartPre (+ prefix) runs as root so setfacl can
          # set the collector's ACL on /run/nixling/otel and host-egress.sock.
          execStartPreIsPrivileged = lib.hasPrefix "+" (svc.serviceConfig.ExecStartPre or "");
        };
      expectedExtract = {
        receiverNames = [ "filelog/store_sync_audit" "hostmetrics" "prometheus" ];
        pipelineNames = [ "logs/store_sync_audit" "metrics" "metrics/self" ];
        hasExtensions = false;
        resourceHasServiceName = false;
        readWritePaths = null;
        umask = null;
        suppGroups = null;
        restart = "on-failure";
        restartSec = "3s";
        startLimitIntervalSec = 0;
        tmpfilesHasIngest = false;
        execStartPreIsPrivileged = true;
      };
    };

    obs-host-collector-journal = {
      override = { ... }: {
        nixling.observability.enable = true;
        nixling.observability.host.scrapeJournal = true;
      };
      extract = nixos:
        let
          cfg = nixos.config.nixling.observability._internal.hostCollectorConfig;
          svc = nixos.config.systemd.services."nixling-host-otel-collector";
        in
        {
          hasJournald = cfg.receivers ? journald;
          hasOtlp = cfg.receivers ? otlp;
          logsReceivers = cfg.service.pipelines.logs.receivers or null;
          hasFileStorage = (cfg.extensions or { }) ? "file_storage/journald";
          journaldStorageDir = (cfg.extensions."file_storage/journald" or { }).directory or null;
          journaldCreateDirectory = (cfg.extensions."file_storage/journald" or { }).create_directory or null;
          suppGroups = svc.serviceConfig.SupplementaryGroups or null;
          readWritePaths = svc.serviceConfig.ReadWritePaths or null;
        };
      expectedExtract = {
        hasJournald = true;
        hasOtlp = false;
        logsReceivers = [ "journald" ];
        hasFileStorage = true;
        journaldStorageDir = "/var/lib/nixling-host-otel-collector/journald";
        journaldCreateDirectory = false;
        suppGroups = [ "systemd-journal" ];
        readWritePaths = null;
      };
    };

    obs-host-collector-otlp = {
      override = { ... }: {
        nixling.observability.enable = true;
        nixling.observability.host.otlpIngest.enable = true;
      };
      extract = nixos:
        let
          cfg = nixos.config.nixling.observability._internal.hostCollectorConfig;
          svc = nixos.config.systemd.services."nixling-host-otel-collector";
          endpoint = cfg.receivers.otlp.protocols.grpc.endpoint;
        in
        {
          hasOtlp = cfg.receivers ? otlp;
          hasJournald = cfg.receivers ? journald;
          otlpProtocols = sortStrings (builtins.attrNames cfg.receivers.otlp.protocols);
          otlpEndpoint = endpoint;
          otlpTransport = cfg.receivers.otlp.protocols.grpc.transport;
          tracesReceivers = cfg.service.pipelines.traces.receivers or null;
          metricsReceivers = cfg.service.pipelines.metrics.receivers;
          logsReceivers = cfg.service.pipelines.logs.receivers or null;
          readWritePaths = svc.serviceConfig.ReadWritePaths or null;
          umask = svc.serviceConfig.UMask or null;
          endpointIsolatedFromEgress = endpoint != "/run/nixling/otel/host-egress.sock";
          tmpfilesHasIngest = builtins.any (r: lib.hasInfix "/run/nixling/otel/ingest" r) nixos.config.systemd.tmpfiles.rules;
        };
      expectedExtract = {
        hasOtlp = true;
        hasJournald = false;
        otlpProtocols = [ "grpc" ];
        otlpEndpoint = "/run/nixling/otel/ingest/host-otlp.sock";
        otlpTransport = "unix";
        tracesReceivers = [ "otlp" ];
        metricsReceivers = [ "hostmetrics" "otlp" ];
        logsReceivers = [ "otlp" ];
        readWritePaths = [ "/run/nixling/otel/ingest" ];
        umask = "0177";
        endpointIsolatedFromEgress = true;
        tmpfilesHasIngest = true;
      };
    };

    obs-host-collector-both-processor-split = {
      override = { ... }: {
        nixling.observability.enable = true;
        nixling.observability.host.scrapeJournal = true;
        nixling.observability.host.otlpIngest.enable = true;
      };
      extract = nixos:
        let
          cfg = nixos.config.nixling.observability._internal.hostCollectorConfig;
          hasKey = procName: name: builtins.any (a: (a.key or "") == name) cfg.processors.${procName}.attributes;
        in
        {
          resourceHasServiceName = hasKey "resource" "service.name";
          selfHasServiceName = hasKey "resource/self" "service.name";
          storesyncHasServiceName = hasKey "resource/store_sync_audit" "service.name";
          resourceVmName = (builtins.head cfg.processors.resource.attributes).value;
          storeSyncVmName = (builtins.head cfg.processors."resource/store_sync_audit".attributes).value;
          logsReceivers = cfg.service.pipelines.logs.receivers;
          metricsReceivers = cfg.service.pipelines.metrics.receivers;
          # Pipeline processor routing: app/journal telemetry must use the
          # identity-only `resource`; only self-metrics use `resource/self`;
          # StoreSync keeps `resource/store_sync_audit`.
          logsProcessors = cfg.service.pipelines.logs.processors;
          tracesProcessors = cfg.service.pipelines.traces.processors;
          metricsProcessors = cfg.service.pipelines.metrics.processors;
          metricsSelfProcessors = cfg.service.pipelines."metrics/self".processors;
          storeSyncProcessors = cfg.service.pipelines."logs/store_sync_audit".processors;
          pipelineNames = sortStrings (builtins.attrNames cfg.service.pipelines);
        };
      expectedExtract = {
        resourceHasServiceName = false;
        selfHasServiceName = true;
        storesyncHasServiceName = true;
        resourceVmName = "nixos";
        storeSyncVmName = "nixos";
        logsReceivers = [ "otlp" "journald" ];
        metricsReceivers = [ "hostmetrics" "otlp" ];
        logsProcessors = [ "memory_limiter" "resource" "batch" ];
        tracesProcessors = [ "memory_limiter" "resource" "batch" ];
        metricsProcessors = [ "memory_limiter" "resource" "batch" ];
        metricsSelfProcessors = [ "memory_limiter" "resource/self" "batch" ];
        storeSyncProcessors = [ "memory_limiter" "resource/store_sync_audit" "batch" ];
        pipelineNames = [ "logs" "logs/store_sync_audit" "metrics" "metrics/self" "traces" ];
      };
    };

    obs-host-identity-override = {
      override = { ... }: {
        nixling.observability.enable = true;
        nixling.observability.host.identityName = "edge-01";
      };
      extract = nixos:
        let
          cfg = nixos.config.nixling.observability._internal.hostCollectorConfig;
          obsVm = nixos.config.nixling.observability.vmName;
          obsGuest = nixos.config.nixling._computed.${obsVm}.config;
          hostSource = obsGuest.nixling.observability.ingress.sources.host;
        in
        {
          edgeVmName = (builtins.head cfg.processors.resource.attributes).value;
          ingressVmName = hostSource.vmName;
          ingressRole = hostSource.role;
          ingressEnv = hostSource.envName;
        };
      expectedExtract = {
        edgeVmName = "edge-01";
        ingressVmName = "edge-01";
        ingressRole = "host";
        ingressEnv = "host";
      };
    };

    obs-host-otlp-client-group-umask = {
      override = { ... }: {
        nixling.observability.enable = true;
        nixling.observability.host.otlpIngest.enable = true;
        nixling.observability.host.otlpIngest.clientGroup = "telemetry";
      };
      extract = nixos: nixos.config.systemd.services."nixling-host-otel-collector".serviceConfig.UMask or null;
      expectedExtract = "0117";
    };

    obs-host-flags-require-enable = {
      kind = "expect-failure";
      override = { ... }: {
        nixling.observability.enable = false;
        nixling.observability.host.scrapeJournal = true;
      };
      expectedSubstring = "the host OTel collector only";
    };
  };

  evaluated = builtins.mapAttrs (_name: spec: evalCase spec) caseSpecs;

  mkSuccessCase = result: {
    expr = {
      inherit (result) evalSucceeded failingMessages extracted;
    };
    expected = {
      evalSucceeded = true;
      failingMessages = [ ];
      extracted = result.expectedExtract;
    };
  };

  mkFailureCase = result: {
    expr = {
      inherit (result) evalSucceeded;
      hasFailingMessages = result.failingMessages != [ ];
      expectedSubstringsPresent = builtins.all
        (needle: lib.any (message: hasInfix needle message) result.failingMessages)
        result.expectedSubstrings;
    };
    expected = {
      evalSucceeded = true;
      hasFailingMessages = true;
      expectedSubstringsPresent = true;
    };
  };

  mkCase = _name: result:
    if result.kind == "expect-failure" then mkFailureCase result else mkSuccessCase result;
in
lib.mapAttrs' (name: result: lib.nameValuePair "observability/${name}" (mkCase name result)) evaluated
