{ flakeRoot }:

let
  shared = import ./shared.nix { inherit flakeRoot; };
  flake = builtins.getFlake (toString flakeRoot);
  nixpkgs = flake.inputs.nixpkgs;
  lib = nixpkgs.lib;

  sortStrings = builtins.sort builtins.lessThan;
  hasInfix = lib.hasInfix;

  forceAttempt = value:
    let
      attempt = builtins.tryEval (builtins.deepSeq value value);
    in
    if attempt.success then
      {
        success = true;
        value = attempt.value;
      }
    else
      {
        success = false;
        value = null;
      };

  evalCase = caseSpec:
    let
      kind = caseSpec.kind or (if caseSpec ? extract || caseSpec ? expectedExtract then "expect-success" else "expect-failure");
      system = caseSpec.system or shared.defaultSystem;
      override = caseSpec.override or ({ ... }: { });
      nixos = nixpkgs.lib.nixosSystem {
        inherit system;
        pkgs = shared.pkgsFor system;
        modules = [
          flake.nixosModules.default
          shared.baseModule
          ({ ... }: { boot.initrd.includeDefaultModules = false; })
          override
        ];
      };
      failureAttempt =
        if kind == "expect-failure" then
          builtins.tryEval (
            let
              assertions = nixos.config.assertions;
              _len = builtins.length assertions;
              _messages = map (assertion: assertion.message) assertions;
            in
            assertions
          )
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
      expectedSubstring = caseSpec.expectedSubstring or "";
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
      aux = auxAttempt.value;
    };

  manifest = nixos: builtins.fromJSON nixos.config.nixling._manifestPkg.text;

  cliPkg =
    nixos:
    builtins.head (
      builtins.filter (
        pkg: (pkg.name or "") == "nixling" || (pkg.pname or "") == "nixling"
      ) nixos.config.environment.systemPackages
    );

  dashboardDir = flakeRoot + "/nixos-modules/components/observability/dashboards";
  dashboardNames =
    sortStrings (
      builtins.filter (
        name: builtins.match ".*\\.json$" name != null
      ) (builtins.attrNames (builtins.readDir dashboardDir))
    );
  dashboardFiles = map (name: dashboardDir + "/${name}") dashboardNames;
  dashboardPaths = map toString dashboardFiles;
  dashboards = map (file: builtins.fromJSON (builtins.readFile file)) dashboardFiles;

  collectDatasourceRefs =
    value:
    if builtins.isAttrs value then
      (lib.optional (value ? datasource && value.datasource != null) value.datasource)
      ++ lib.concatMap collectDatasourceRefs (builtins.attrValues value)
    else if builtins.isList value then
      lib.concatMap collectDatasourceRefs value
    else
      [ ];

  dashboardHasRequiredShape =
    dashboard:
    builtins.typeOf (dashboard.uid or null) == "string"
    && builtins.typeOf (dashboard.title or null) == "string"
    && builtins.typeOf (dashboard.schemaVersion or null) == "int"
    && builtins.isList (dashboard.panels or [ ])
    && (dashboard.panels or [ ]) != [ ];

  dashboardDatasourceRefsOk =
    dashboard:
    builtins.all (
      ref:
      builtins.isAttrs ref
      && builtins.elem (ref.uid or "") [ "prometheus" "loki" "tempo" ]
      && builtins.elem (ref.type or "") [ "prometheus" "loki" "tempo" ]
    ) (collectDatasourceRefs dashboard);

  mkCase = spec: evalCase spec;
in
{
  obs-disabled-default = mkCase {
    extract = nixos: (manifest nixos)._observability.enabled;
    expectedExtract = false;
  };

  obs-default-off-no-units = mkCase {
    override = { ... }: { nixling.observability.enable = false; };
    extract = nixos: {
      otelServiceNames = sortStrings (
        builtins.filter (name: builtins.match "^nixling-otel-.*" name != null) (builtins.attrNames nixos.config.systemd.services)
      );
    };
    expectedExtract = { otelServiceNames = [ ]; };
    aux = nixos: { cliDrvPath = (cliPkg nixos).drvPath; };
  };

  obs-enabled-defaults = mkCase {
    override = { ... }: { nixling.observability.enable = true; };
    extract = nixos:
      let
        manifestData = manifest nixos;
        obsVm = nixos.config.nixling.observability.vmName;
        obsEnv = lib.attrByPath [ "obs" ] { } nixos.config.nixling.envs;
      in
      {
        hasSysObsStack = builtins.hasAttr "sys-obs-stack" nixos.config.nixling.vms;
        hasObsEnv = builtins.hasAttr "obs" nixos.config.nixling.envs;
        obsEnvLanSubnet = obsEnv.lanSubnet or null;
        obsEnvUplinkSubnet = obsEnv.uplinkSubnet or null;
        obsVmName = lib.attrByPath [ "_observability" "vmName" ] null manifestData;
        obsVsockCid = lib.attrByPath [ "_observability" "obsVsockCid" ] null manifestData;
        grafanaListenAddress = nixos.config.nixling.observability.grafana.listenAddress;
        obsVmStaticIp = lib.attrByPath [ obsVm "staticIp" ] null nixos.config.nixling.manifest;
        grafanaUrl = lib.attrByPath [ "_observability" "grafanaUrl" ] null manifestData;
      };
    expectedExtract = {
      hasSysObsStack = true;
      hasObsEnv = true;
      obsEnvLanSubnet = "10.40.0.0/24";
      obsEnvUplinkSubnet = "203.0.113.0/30";
      obsVmName = "sys-obs-stack";
      obsVsockCid = 1000;
      grafanaListenAddress = "10.40.0.10";
      obsVmStaticIp = "10.40.0.10";
      grafanaUrl = "http://10.40.0.10:3000";
    };
  };

  obs-grafana-bind-tracks-obs-ip = mkCase {
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
        grafanaListenAddress = nixos.config.nixling.observability.grafana.listenAddress;
        obsVmStaticIp = lib.attrByPath [ obsVm "staticIp" ] null nixos.config.nixling.manifest;
        grafanaUrl = lib.attrByPath [ "_observability" "grafanaUrl" ] null manifestData;
      };
    expectedExtract = {
      grafanaListenAddress = "10.44.0.23";
      obsVmStaticIp = "10.44.0.23";
      grafanaUrl = "http://10.44.0.23:3000";
    };
  };

  obs-name-extension-allowed = mkCase {
    override = { ... }: {
      nixling.observability.enable = true;
      nixling.vms.sys-obs-stack = {
        ssh.user = "alice";
        config.users.users.alice = { isNormalUser = true; uid = 1000; };
      };
    };
    extract = nixos: builtins.hasAttr "sys-obs-stack" nixos.config.nixling.vms;
    expectedExtract = true;
  };

  obs-cid-collision = mkCase {
    kind = "expect-failure";
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
    expectedSubstring = "Vsock CID collision:";
    expectedSubstrings = [ "CID" "corp-vm" "other-vm" ];
  };

  obs-manifest-fields = mkCase {
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
      vsockCid = 210;
      vsockHostSocket = "/var/lib/nixling/vms/corp-vm/vsock.sock";
      agentSocket = "/run/nixling/otlp.sock";
    };
  };

  obs-relay-acl-surface = mkCase {
    override = { ... }: {
      nixling.observability.enable = true;
      nixling.vms.corp-vm.observability.enable = true;
    };
    extract = nixos:
      let
        relay = nixos.config.systemd.services."nixling-otel-relay@";
        execStartPre = builtins.head relay.serviceConfig.ExecStartPre;
      in
      {
        relayGroupDeclared = builtins.hasAttr "nixling-otel-relay" nixos.config.users.groups;
        relayUserDeclared = builtins.hasAttr "nixling-otel-relay" nixos.config.users.users;
        relayUserGroup = nixos.config.users.users.nixling-otel-relay.group;
        relayServiceUser = relay.serviceConfig.User;
        relayServiceGroup = relay.serviceConfig.Group;
        relayDynamicUser = relay.serviceConfig.DynamicUser;
        relaySupplementaryGroups = relay.serviceConfig.SupplementaryGroups or [ ];
        relayExecStartPreHasAclRefresh = hasInfix "nixling-otel-acl-refresh" execStartPre;
        relayExecStartPreMatchesActivationPath =
          lib.removePrefix "+" execStartPre == nixos.config.system.activationScripts.nixlingOtelSocketAcls.text;
      };
    expectedExtract = {
      relayGroupDeclared = true;
      relayUserDeclared = true;
      relayUserGroup = "nixling-otel-relay";
      relayServiceUser = "nixling-otel-relay";
      relayServiceGroup = "nixling-otel-relay";
      relayDynamicUser = false;
      relaySupplementaryGroups = [ ];
      relayExecStartPreHasAclRefresh = true;
      relayExecStartPreMatchesActivationPath = true;
    };
  };

  obs-stack-vm-guest-surface = mkCase {
    override = { ... }: {
      nixling.observability.enable = true;
      nixling.observability.retention.metrics = "5d";
      nixling.observability.retention.logs = "3d";
      nixling.observability.retention.traces = "1d";
    };
    extract = nixos:
      let
        obsVm = nixos.config.nixling.observability.vmName;
        obsGuest = nixos.config.microvm.vms.${obsVm}.config.config;
        grafanaDatasources = obsGuest.services.grafana.provision.datasources.settings.datasources;
        dashboardProviders = obsGuest.services.grafana.provision.dashboards.settings.providers;
        lokiDatasource = builtins.head (builtins.filter (ds: ds.name == "Loki") grafanaDatasources);
        tempoDatasource = builtins.head (builtins.filter (ds: ds.name == "Tempo") grafanaDatasources);
      in
      {
        obsVmName = obsVm;
        manifestHasObsVm = builtins.hasAttr obsVm nixos.config.nixling.manifest;
        grafanaEnable = obsGuest.services.grafana.enable;
        prometheusEnable = obsGuest.services.prometheus.enable;
        lokiEnable = obsGuest.services.loki.enable;
        tempoEnable = obsGuest.services.tempo.enable;
        alloyEnable = obsGuest.services.alloy.enable;
        vsockInDeclared = builtins.hasAttr "nixling-otel-vsock-in" obsGuest.systemd.services;
        vsockInRestartIfChanged = obsGuest.systemd.services.nixling-otel-vsock-in.restartIfChanged;
        grafanaLoadCredentialHasSecretKey = builtins.elem
          "secret_key:/run/nixling-obs-secrets/grafana-secret-key"
          (obsGuest.systemd.services.grafana.serviceConfig.LoadCredential or [ ]);
        grafanaSecretKey = obsGuest.services.grafana.settings.security.secret_key;
        datasourceUrls = builtins.listToAttrs (map (ds: { name = ds.uid or (lib.toLower ds.name); value = ds.url; }) grafanaDatasources);
        lokiDerivedFieldsCount = builtins.length (lokiDatasource.jsonData.derivedFields or [ ]);
        tempoTraceToLogsDatasource = tempoDatasource.jsonData.tracesToLogsV2.datasourceUid or null;
        dashboardProviderCount = builtins.length dashboardProviders;
        dashboardFolder = (builtins.head dashboardProviders).folder or null;
        dashboardPathHasDashboards = hasInfix "nixling-grafana-dashboards" ((builtins.head dashboardProviders).options.path or "");
        prometheusRetention = obsGuest.services.prometheus.retentionTime;
        lokiRetention = obsGuest.services.loki.configuration.limits_config.retention_period;
        tempoRetention = obsGuest.services.tempo.settings.compactor.compaction.block_retention;
        grafanaBindAddress = obsGuest.services.grafana.settings.server.http_addr;
        vsockInExecStartHasShape = hasInfix
          "bin/socat -d -d VSOCK-LISTEN:14317,fork,max-children=16,reuseaddr UNIX-CONNECT:/run/nixling/obs-ingress.sock"
          obsGuest.systemd.services.nixling-otel-vsock-in.serviceConfig.ExecStart;
      };
    expectedExtract = {
      obsVmName = "sys-obs-stack";
      manifestHasObsVm = true;
      grafanaEnable = true;
      prometheusEnable = true;
      lokiEnable = true;
      tempoEnable = true;
      alloyEnable = true;
      vsockInDeclared = true;
      vsockInRestartIfChanged = false;
      grafanaLoadCredentialHasSecretKey = true;
      grafanaSecretKey = "$__file{/run/credentials/grafana.service/secret_key}";
      datasourceUrls = {
        loki = "http://127.0.0.1:3100";
        prometheus = "http://127.0.0.1:9090";
        tempo = "http://127.0.0.1:3200";
      };
      lokiDerivedFieldsCount = 1;
      tempoTraceToLogsDatasource = "loki";
      dashboardProviderCount = 1;
      dashboardFolder = "Nixling";
      dashboardPathHasDashboards = true;
      prometheusRetention = "5d";
      lokiRetention = "3d";
      tempoRetention = "1d";
      grafanaBindAddress = "10.40.0.10";
      vsockInExecStartHasShape = true;
    };
  };

  obs-alerting-surface = mkCase {
    override = { ... }: {
      nixling.observability.enable = true;
      nixling.vms.corp-vm.observability.enable = true;
    };
    extract = nixos:
      let
        obsVm = nixos.config.nixling.observability.vmName;
        obsGuest = nixos.config.microvm.vms.${obsVm}.config.config;
        workGuest = nixos.config.microvm.vms.corp-vm.config.config;
        ruleFiles = obsGuest.services.prometheus.ruleFiles;
        workGuestAlloyConfig = workGuest.environment.etc."alloy/config.alloy".text;
      in
      {
        ruleFileCount = builtins.length ruleFiles;
        ruleFileNameMatches = ruleFiles != [ ] && hasInfix "nixling-observability.rules.yml" (toString (builtins.head ruleFiles));
        obsPrometheusJobs = sortStrings (map (scrape: scrape.job_name) obsGuest.services.prometheus.scrapeConfigs);
        guestAlloyHasTelemetryJob = hasInfix "job_name   = \"nixling-vm-telemetry\"" workGuestAlloyConfig;
        guestAlloyHasNodeJob = hasInfix "job_name   = \"nixling-vm-node\"" workGuestAlloyConfig;
        guestAlloyHasVmLabel = hasInfix "target_label = \"vm\"" workGuestAlloyConfig;
        guestAlloyHasEnvLabel = hasInfix "target_label = \"env\"" workGuestAlloyConfig;
      };
    expectedExtract = {
      ruleFileCount = 1;
      ruleFileNameMatches = true;
      obsPrometheusJobs = [ "alloy" "grafana" "loki" "prometheus" "tempo" ];
      guestAlloyHasTelemetryJob = true;
      guestAlloyHasNodeJob = true;
      guestAlloyHasVmLabel = true;
      guestAlloyHasEnvLabel = true;
    };
    aux = nixos:
      let
        obsVm = nixos.config.nixling.observability.vmName;
        obsGuest = nixos.config.microvm.vms.${obsVm}.config.config;
      in
      {
        rulesPath = toString (builtins.head obsGuest.services.prometheus.ruleFiles);
      };
  };

  obs-vm-toggle-default-off = mkCase {
    override = { ... }: { nixling.observability.enable = true; };
    extract = nixos: lib.attrByPath [ "corp-vm" "observability" "enabled" ] null (manifest nixos);
    expectedExtract = false;
  };

  obs-audit-surface = mkCase {
    override = { ... }: {
      nixling.observability.enable = true;
      nixling.vms.corp-vm.observability = {
        enable = true;
        scrapeJournal = false;
      };
      nixling.vms.corp-vm.audit.enable = true;
    };
    extract = nixos:
      let
        workGuest = nixos.config.microvm.vms.corp-vm.config.config;
        workGuestAlloyConfig = workGuest.environment.etc."alloy/config.alloy".text;
      in
      {
        auditEnabled = workGuest.security.audit.enable;
        auditdEnabled = workGuest.security.auditd.enable;
        auditdSyslogPlugin = workGuest.security.auditd.plugins.syslog.active;
        alloyHasJournalGroup = builtins.elem "systemd-journal" (workGuest.systemd.services.alloy.serviceConfig.SupplementaryGroups or [ ]);
        auditRules = sortStrings workGuest.security.audit.rules;
        alloyHasJournalReceiver = hasInfix "otelcol.receiver.loki \"journal\"" workGuestAlloyConfig;
        alloyHasAuditJournalSource = hasInfix "loki.source.journal \"audit\"" workGuestAlloyConfig;
        alloyHasAudispUnitLabel = hasInfix "unit   = \"audisp-syslog\"" workGuestAlloyConfig;
        alloyHasAudispMatch = hasInfix "matches    = \"_TRANSPORT=syslog SYSLOG_IDENTIFIER=audisp-syslog\"" workGuestAlloyConfig;
        alloyHasGeneralJournalSource = hasInfix "loki.source.journal \"journal\"" workGuestAlloyConfig;
      };
    expectedExtract = {
      auditEnabled = true;
      auditdEnabled = true;
      auditdSyslogPlugin = true;
      alloyHasJournalGroup = true;
      auditRules = [
        "-w /etc/passwd -p wa -k identity"
        "-w /etc/shadow -p wa -k identity"
        "-w /etc/sudoers -p wa -k priv-esc"
      ];
      alloyHasJournalReceiver = true;
      alloyHasAuditJournalSource = true;
      alloyHasAudispUnitLabel = true;
      alloyHasAudispMatch = true;
      alloyHasGeneralJournalSource = false;
    };
  };

  obs-cli-traces-default-on = mkCase {
    override = { ... }: { nixling.observability.enable = true; };
    extract = nixos: nixos.config.nixling.observability.cli.traces.enable;
    expectedExtract = true;
    aux = nixos: { cliDrvPath = (cliPkg nixos).drvPath; };
  };

  obs-cli-traces-disabled = mkCase {
    override = { ... }: {
      nixling.observability.enable = true;
      nixling.observability.cli.traces.enable = false;
    };
    extract = nixos: nixos.config.nixling.observability.cli.traces.enable;
    expectedExtract = false;
    aux = nixos: { cliDrvPath = (cliPkg nixos).drvPath; };
  };

  obs-cli-trace-attr-allowlist = mkCase {
    override = { ... }: { nixling.observability.enable = true; };
    extract = _nixos: true;
    expectedExtract = true;
    aux = nixos: { cliDrvPath = (cliPkg nixos).drvPath; };
  };

  obs-reserved-prefix-exempt = mkCase {
    override = { ... }: { nixling.observability.enable = true; };
    extract = nixos: builtins.hasAttr "sys-obs-stack" nixos.config.nixling.vms;
    expectedExtract = true;
  };

  obs-vm-without-framework = mkCase {
    kind = "expect-failure";
    override = { ... }: { nixling.vms.corp-vm.observability.enable = true; };
    expectedSubstring = "observability.enable = true but nixling.observability.enable is false";
  };

  obs-dashboards-schema = mkCase {
    extract = _nixos: {
      dashboardFileCount = builtins.length dashboardPaths;
      requiredShape = builtins.all dashboardHasRequiredShape dashboards;
      datasourceRefsOk = builtins.all dashboardDatasourceRefsOk dashboards;
    };
    expectedExtract = {
      dashboardFileCount = 6;
      requiredShape = true;
      datasourceRefsOk = true;
    };
    aux = _nixos: { inherit dashboardPaths; };
  };

  obs-rules-promtool = mkCase {
    override = { ... }: {
      nixling.observability.enable = true;
      nixling.vms.corp-vm.observability.enable = true;
    };
    extract = nixos:
      let
        obsVm = nixos.config.nixling.observability.vmName;
        obsGuest = nixos.config.microvm.vms.${obsVm}.config.config;
      in
      builtins.length obsGuest.services.prometheus.ruleFiles;
    expectedExtract = 1;
    aux = nixos:
      let
        obsVm = nixos.config.nixling.observability.vmName;
        obsGuest = nixos.config.microvm.vms.${obsVm}.config.config;
      in
      {
        rulesPath = toString (builtins.head obsGuest.services.prometheus.ruleFiles);
      };
  };

  obs-metric-references = mkCase {
    override = { ... }: {
      nixling.observability.enable = true;
      nixling.vms.corp-vm.observability.enable = true;
    };
    extract = nixos:
      let
        obsVm = nixos.config.nixling.observability.vmName;
        obsGuest = nixos.config.microvm.vms.${obsVm}.config.config;
      in
      {
        dashboardFileCount = builtins.length dashboardPaths;
        ruleFileCount = builtins.length obsGuest.services.prometheus.ruleFiles;
      };
    expectedExtract = {
      dashboardFileCount = 6;
      ruleFileCount = 1;
    };
  };

  obs-scrape-job-stability = mkCase {
    override = { ... }: {
      nixling.observability.enable = true;
      nixling.vms.corp-vm.observability.enable = true;
    };
    extract = nixos:
      let
        obsVm = nixos.config.nixling.observability.vmName;
        obsGuest = nixos.config.microvm.vms.${obsVm}.config.config;
      in
      sortStrings (map (scrape: scrape.job_name) obsGuest.services.prometheus.scrapeConfigs);
    expectedExtract = [ "alloy" "grafana" "loki" "prometheus" "tempo" ];
    aux = nixos: { hostAlloyConfigPath = toString nixos.config.services.alloy.configPath; };
  };

  obs-stability = mkCase {
    override = { ... }: {
      nixling.observability.enable = true;
      nixling.vms.corp-vm.observability.enable = true;
    };
    extract = nixos:
      let
        obsVm = nixos.config.nixling.observability.vmName;
        obsGuest = nixos.config.microvm.vms.${obsVm}.config.config;
        grafanaDatasources = obsGuest.services.grafana.provision.datasources.settings.datasources;
      in
      {
        datasourceUids = sortStrings (map (ds: ds.uid or "") grafanaDatasources);
        obsPrometheusJobs = sortStrings (map (scrape: scrape.job_name) obsGuest.services.prometheus.scrapeConfigs);
      };
    expectedExtract = {
      datasourceUids = [ "loki" "prometheus" "tempo" ];
      obsPrometheusJobs = [ "alloy" "grafana" "loki" "prometheus" "tempo" ];
    };
    aux = nixos:
      let
        obsVm = nixos.config.nixling.observability.vmName;
        obsGuest = nixos.config.microvm.vms.${obsVm}.config.config;
      in
      {
        rulesPath = toString (builtins.head obsGuest.services.prometheus.ruleFiles);
        hostAlloyConfigPath = toString nixos.config.services.alloy.configPath;
      };
  };

  obs-graphics-runner-wiring = mkCase {
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
        relay = nixos.config.systemd.services."nixling-otel-relay@";
        gpuUnit = nixos.config.systemd.services."nixling-gpu-vm-gpu" or null;
      in
      {
        relayBindsToHasMicrovm = builtins.elem "microvm@%i.service" (relay.bindsTo or [ ]);
        gpuServiceDeclared = gpuUnit != null;
        gpuWantsRelay = if gpuUnit == null then null else builtins.elem "nixling-otel-relay@gpu-vm.service" (gpuUnit.wants or [ ]);
        relayExecConditionHasEligibility = hasInfix "nixling-otel-relay-eligible" (relay.serviceConfig.ExecCondition or "");
        relayExecStartPreGatesOnVsockSock = builtins.any (cmd: hasInfix "vsock.sock" cmd) (relay.serviceConfig.ExecStartPre or [ ]);
      };
    expectedExtract = {
      relayBindsToHasMicrovm = false;
      gpuServiceDeclared = true;
      gpuWantsRelay = true;
      relayExecConditionHasEligibility = true;
      relayExecStartPreGatesOnVsockSock = true;
    };
  };
}
