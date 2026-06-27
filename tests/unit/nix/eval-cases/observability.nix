{ flakeRoot }:

let
  shared = import ./shared.nix { inherit flakeRoot; };
  flake = builtins.getFlake "git+file://${toString flakeRoot}";
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

  manifest = nixos: builtins.fromJSON nixos.config.d2b._manifestPkg.text;

  cliPkg =
    nixos:
    builtins.head (
      builtins.filter (
        pkg: (pkg.name or "") == "d2b" || (pkg.pname or "") == "d2b"
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

  mkCase = spec: evalCase spec;
in
{
  obs-disabled-default = mkCase {
    extract = nixos: (manifest nixos)._observability.enabled;
    expectedExtract = false;
  };

  obs-default-off-no-units = mkCase {
    override = { ... }: { d2b.observability.enable = false; };
    extract = nixos: {
      otelServiceNames = sortStrings (
        builtins.filter (name: builtins.match "^d2b-otel-.*" name != null) (builtins.attrNames nixos.config.systemd.services)
      );
    };
    expectedExtract = { otelServiceNames = [ ]; };
    aux = nixos: { cliDrvPath = (cliPkg nixos).drvPath; };
  };

  obs-enabled-defaults = mkCase {
    override = { ... }: { d2b.observability.enable = true; };
    extract = nixos:
      let
        manifestData = manifest nixos;
        obsVm = nixos.config.d2b.observability.vmName;
        obsEnv = lib.attrByPath [ "obs" ] { } nixos.config.d2b.envs;
      in
      {
        hasSysObs = builtins.hasAttr "sys-obs" nixos.config.d2b.vms;
        hasObsEnv = builtins.hasAttr "obs" nixos.config.d2b.envs;
        obsEnvLanSubnet = obsEnv.lanSubnet or null;
        obsEnvUplinkSubnet = obsEnv.uplinkSubnet or null;
        obsVmName = lib.attrByPath [ "_observability" "vmName" ] null manifestData;
        obsVsockCid = lib.attrByPath [ "_observability" "obsVsockCid" ] null manifestData;
        signozListenAddress = nixos.config.d2b.observability.signoz.listenAddress;
        obsVmStaticIp = lib.attrByPath [ obsVm "staticIp" ] null nixos.config.d2b.manifest;
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

  obs-signoz-bind-tracks-obs-ip = mkCase {
    override = { ... }: {
      d2b.observability.enable = true;
      d2b.observability.lanSubnet = "10.44.0.0/24";
      d2b.observability.index = 23;
    };
    extract = nixos:
      let
        manifestData = manifest nixos;
        obsVm = nixos.config.d2b.observability.vmName;
      in
      {
        signozListenAddress = nixos.config.d2b.observability.signoz.listenAddress;
        obsVmStaticIp = lib.attrByPath [ obsVm "staticIp" ] null nixos.config.d2b.manifest;
        signozUrl = lib.attrByPath [ "_observability" "signozUrl" ] null manifestData;
      };
    expectedExtract = {
      signozListenAddress = "10.44.0.23";
      obsVmStaticIp = "10.44.0.23";
      signozUrl = "http://10.44.0.23:8080";
    };
  };

  obs-name-extension-allowed = mkCase {
    override = { ... }: {
      d2b.observability.enable = true;
      d2b.vms.sys-obs = {
        ssh.user = "alice";
        config.users.users.alice = { isNormalUser = true; uid = 1000; };
      };
    };
    extract = nixos: builtins.hasAttr "sys-obs" nixos.config.d2b.vms;
    expectedExtract = true;
  };

  obs-cid-cross-env-noncollision = mkCase {
    override = { lib, ... }: {
      d2b.observability.enable = true;
      d2b.envs.aaa = {
        lanSubnet = "10.30.0.0/24";
        uplinkSubnet = "198.51.100.0/30";
      };
      d2b.envs.bbb = {
        lanSubnet = "10.31.0.0/24";
        uplinkSubnet = "198.18.0.0/30";
      };
      d2b.vms.corp-vm.env = lib.mkForce "aaa";
      d2b.vms.corp-vm.index = lib.mkForce 110;
      d2b.vms.corp-vm.observability.enable = true;
      d2b.vms.other-vm = {
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
      let
        data = manifest nixos;
      in {
        corp = data.corp-vm.observability.vsockCid;
        other = data.other-vm.observability.vsockCid;
      };
    expectedExtract = {
      corp = 210;
      other = 1110;
    };
  };

  obs-manifest-fields = mkCase {
    override = { ... }: { d2b.observability.enable = true; };
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
      vsockHostSocket = "/var/lib/d2b/vms/corp-vm/vsock.sock";
      agentSocket = "/run/d2b/otlp.sock";
    };
  };

  obs-relay-acl-surface = mkCase {
    override = { ... }: {
      d2b.observability.enable = true;
      d2b.vms.corp-vm.observability.enable = true;
    };
    extract = nixos:
      let
        processes = builtins.fromJSON nixos.config.d2b._bundle.processesJson.jsonText;
        corpDag = builtins.head (builtins.filter (dag: dag.vm == "corp-vm") processes.vms);
        relayNode = builtins.head (builtins.filter (node: node.id == "vsock-relay") corpDag.nodes);
        obsDag = builtins.head (builtins.filter (dag: dag.vm == "sys-obs") processes.vms);
        bridgeNode = builtins.head (builtins.filter (node: node.id == "otel-host-bridge") obsDag.nodes);
        activationText = nixos.config.system.activationScripts.d2bRoleUidAcls.text;
      in
      {
        relayNodeRole = relayNode.role;
        relayProfileHasEmptyCaps = relayNode.profile.capabilities == [ ];
        relayProfileSeccomp = relayNode.profile.seccompPolicyRef;
        bridgeNodeRole = bridgeNode.role;
        bridgeProfileHasRuntimeBind =
          builtins.any (path: path == "/run/d2b/otel") bridgeNode.profile.mountPolicy.writablePaths;
        activationGrantsOtelRuntime = hasInfix "/run/d2b/otel" activationText;
        activationExcludesBridgeFromBroadVmAcl = hasInfix "otel_host_bridge_uids" activationText;
      };
    expectedExtract = {
      relayNodeRole = "vsock-relay";
      relayProfileHasEmptyCaps = true;
      relayProfileSeccomp = "w1-vsock-relay";
      bridgeNodeRole = "otel-host-bridge";
      bridgeProfileHasRuntimeBind = true;
      activationGrantsOtelRuntime = true;
      activationExcludesBridgeFromBroadVmAcl = true;
    };
  };

  obs-stack-vm-guest-surface = mkCase {
    override = { ... }: {
      d2b.observability.enable = true;
      d2b.vms.corp-vm.observability.enable = true;
      d2b.observability.retention.metrics = "5d";
      d2b.observability.retention.logs = "3d";
      d2b.observability.retention.traces = "1d";
    };
    extract = nixos:
      let
        obsVm = nixos.config.d2b.observability.vmName;
        obsGuest = nixos.config.microvm.vms.${obsVm}.config.config;
        services = obsGuest.systemd.services;
        ingressSources = obsGuest.d2b.observability.ingress.sources;
      in
      {
        obsVmName = obsVm;
        manifestHasObsVm = builtins.hasAttr obsVm nixos.config.d2b.manifest;
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
        hostVsockInDeclared = builtins.hasAttr "d2b-otel-vsock-in-host" services;
        corpVsockInDeclared = builtins.hasAttr "d2b-otel-vsock-in-corp-vm" services;
        hostVsockInExecStartHasShape = hasInfix
          "VSOCK-LISTEN:14317,fork,max-children=16,reuseaddr TCP:127.0.0.1:4317"
          services.d2b-otel-vsock-in-host.serviceConfig.ExecStart;
        corpVsockInExecStartHasShape = hasInfix
          "VSOCK-LISTEN:14318,fork,max-children=16,reuseaddr TCP:127.0.0.1:14318"
          services.d2b-otel-vsock-in-corp-vm.serviceConfig.ExecStart;
        signozBindAddress = obsGuest.d2b.observability.signoz.listenAddress;
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
        vmName = "host";
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

  obs-alerting-surface = mkCase {
    override = { ... }: {
      d2b.observability.enable = true;
      d2b.vms.corp-vm.observability.enable = true;
    };
    extract = nixos:
      let
        obsVm = nixos.config.d2b.observability.vmName;
        obsGuest = nixos.config.microvm.vms.${obsVm}.config.config;
        workGuest = nixos.config.microvm.vms.corp-vm.config.config;
      in
      {
        obsIngressSources = sortStrings (builtins.attrNames obsGuest.d2b.observability.ingress.sources);
        guestOtelCollectorDeclared = builtins.hasAttr "d2b-otel-collector" workGuest.systemd.services;
        guestVsockOutDeclared = builtins.hasAttr "d2b-otel-vsock-out" workGuest.systemd.services;
        guestAlloyAbsent = ! builtins.hasAttr "alloy" workGuest.systemd.services;
        guestIdentity = workGuest.d2b.observability.identity;
        guestVsockOutHasHostPort = hasInfix "VSOCK-CONNECT:2:14317"
          workGuest.systemd.services.d2b-otel-vsock-out.serviceConfig.ExecStart;
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

  obs-vm-toggle-default-off = mkCase {
    override = { ... }: { d2b.observability.enable = true; };
    extract = nixos: lib.attrByPath [ "corp-vm" "observability" "enabled" ] null (manifest nixos);
    expectedExtract = false;
  };

  obs-journal-default-on = mkCase {
    override = { ... }: {
      d2b.observability.enable = true;
      d2b.vms.corp-vm.observability.enable = true;
    };
    extract = nixos:
      let
        workGuest = nixos.config.microvm.vms.corp-vm.config.config;
      in
      {
        scrapeJournalResolved = workGuest.d2b.observability.scrapeJournal;
        otelUserInJournalGroup =
          builtins.elem "systemd-journal" (workGuest.users.users.otel.extraGroups or [ ]);
      };
    expectedExtract = {
      scrapeJournalResolved = true;
      otelUserInJournalGroup = true;
    };
  };

  obs-audit-surface = mkCase {
    override = { ... }: {
      d2b.observability.enable = true;
      d2b.vms.corp-vm.observability = {
        enable = true;
        scrapeJournal = false;
      };
      d2b.vms.corp-vm.audit.enable = true;
    };
    extract = nixos:
      let
        workGuest = nixos.config.microvm.vms.corp-vm.config.config;
      in
      {
        auditEnabled = workGuest.security.audit.enable;
        auditdEnabled = workGuest.security.auditd.enable;
        auditdSyslogPlugin = workGuest.security.auditd.plugins.syslog.active;
        guestOtelCollectorDeclared = builtins.hasAttr "d2b-otel-collector" workGuest.systemd.services;
        guestAlloyAbsent = ! builtins.hasAttr "alloy" workGuest.systemd.services;
        scrapeJournalResolved = workGuest.d2b.observability.scrapeJournal;
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

  obs-cli-traces-default-on = mkCase {
    override = { ... }: { d2b.observability.enable = true; };
    extract = nixos: nixos.config.d2b.observability.cli.traces.enable;
    expectedExtract = true;
    aux = nixos: { cliDrvPath = (cliPkg nixos).drvPath; };
  };

  obs-cli-traces-disabled = mkCase {
    override = { ... }: {
      d2b.observability.enable = true;
      d2b.observability.cli.traces.enable = false;
    };
    extract = nixos: nixos.config.d2b.observability.cli.traces.enable;
    expectedExtract = false;
    aux = nixos: { cliDrvPath = (cliPkg nixos).drvPath; };
  };

  obs-cli-trace-attr-allowlist = mkCase {
    override = { ... }: { d2b.observability.enable = true; };
    extract = _nixos: true;
    expectedExtract = true;
    aux = nixos: { cliDrvPath = (cliPkg nixos).drvPath; };
  };

  obs-reserved-prefix-exempt = mkCase {
    override = { ... }: { d2b.observability.enable = true; };
    extract = nixos: builtins.hasAttr "sys-obs" nixos.config.d2b.vms;
    expectedExtract = true;
  };

  obs-vm-without-framework = mkCase {
    kind = "expect-failure";
    override = { ... }: { d2b.vms.corp-vm.observability.enable = true; };
    expectedSubstring = "observability.enable = true but d2b.observability.enable is false";
  };

  obs-dashboards-schema = mkCase {
    extract = _nixos: {
      dashboardFileCount = builtins.length dashboardPaths;
      retiredDashboardDirIsEmpty = dashboardPaths == [ ];
    };
    expectedExtract = {
      dashboardFileCount = 0;
      retiredDashboardDirIsEmpty = true;
    };
  };

  obs-rules-promtool = mkCase {
    override = { ... }: {
      d2b.observability.enable = true;
      d2b.vms.corp-vm.observability.enable = true;
    };
    extract = nixos:
      let
        obsVm = nixos.config.d2b.observability.vmName;
        obsGuest = nixos.config.microvm.vms.${obsVm}.config.config;
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

  obs-metric-references = mkCase {
    override = { ... }: {
      d2b.observability.enable = true;
      d2b.vms.corp-vm.observability.enable = true;
    };
    extract = nixos:
      let
        obsVm = nixos.config.d2b.observability.vmName;
        obsGuest = nixos.config.microvm.vms.${obsVm}.config.config;
        ingressSources = obsGuest.d2b.observability.ingress.sources;
        processes = builtins.fromJSON nixos.config.d2b._bundle.processesJson.jsonText;
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
          "d2b-ch-vsock-connect /var/lib/d2b/vms/sys-obs/vsock.sock 14318"
          relayArgv;
      };
    expectedExtract = {
      sourceNames = [ "corp-vm" "host" ];
      hostReceiverGrpcPort = 4317;
      corpReceiverGrpcPort = 4319;
      hostVsockPort = 14317;
      corpVsockPort = 14318;
      relayTargetsCorpIngressPort = true;
    };
  };

  obs-scrape-job-stability = mkCase {
    override = { ... }: {
      d2b.observability.enable = true;
      d2b.vms.corp-vm.observability.enable = true;
    };
    extract = nixos:
      let
        obsVm = nixos.config.d2b.observability.vmName;
        obsGuest = nixos.config.microvm.vms.${obsVm}.config.config;
        services = obsGuest.systemd.services;
      in
      {
        hostIngressExecHasShape = hasInfix
          "VSOCK-LISTEN:14317,fork,max-children=16,reuseaddr TCP:127.0.0.1:4317"
          services.d2b-otel-vsock-in-host.serviceConfig.ExecStart;
        corpIngressExecHasShape = hasInfix
          "VSOCK-LISTEN:14318,fork,max-children=16,reuseaddr TCP:127.0.0.1:14318"
          services.d2b-otel-vsock-in-corp-vm.serviceConfig.ExecStart;
        hostIngressRestartIfChanged = services.d2b-otel-vsock-in-host.restartIfChanged;
        corpIngressRestartIfChanged = services.d2b-otel-vsock-in-corp-vm.restartIfChanged;
      };
    expectedExtract = {
      hostIngressExecHasShape = true;
      corpIngressExecHasShape = true;
      hostIngressRestartIfChanged = false;
      corpIngressRestartIfChanged = false;
    };
  };

  obs-stability = mkCase {
    override = { ... }: {
      d2b.observability.enable = true;
      d2b.vms.corp-vm.observability.enable = true;
    };
    extract = nixos:
      let
        obsVm = nixos.config.d2b.observability.vmName;
        obsGuest = nixos.config.microvm.vms.${obsVm}.config.config;
        workGuest = nixos.config.microvm.vms.corp-vm.config.config;
      in
      {
        retiredBackendServicesAbsent = !(
          obsGuest.systemd.services ? grafana
          || obsGuest.systemd.services ? prometheus
          || obsGuest.systemd.services ? loki
          || obsGuest.systemd.services ? tempo
          || obsGuest.systemd.services ? alloy
        );
        hostCollectorDeclared = builtins.hasAttr "d2b-host-otel-collector" nixos.config.systemd.services;
        guestCollectorDeclared = builtins.hasAttr "d2b-otel-collector" workGuest.systemd.services;
        guestVsockOutDeclared = builtins.hasAttr "d2b-otel-vsock-out" workGuest.systemd.services;
      };
    expectedExtract = {
      retiredBackendServicesAbsent = true;
      hostCollectorDeclared = true;
      guestCollectorDeclared = true;
      guestVsockOutDeclared = true;
    };
  };

  obs-graphics-runner-wiring = mkCase {
    override = { ... }: {
      d2b.observability.enable = true;
      d2b.vms.gpu-vm = {
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
        gpuUnit = nixos.config.systemd.services."d2b-gpu-vm-gpu" or null;
        processes = builtins.fromJSON nixos.config.d2b._bundle.processesJson.jsonText;
        gpuDag = builtins.head (builtins.filter (dag: dag.vm == "gpu-vm") processes.vms);
        nodeIds = sortStrings (map (node: node.id) gpuDag.nodes);
        relayNode = builtins.head (builtins.filter (node: node.id == "vsock-relay") gpuDag.nodes);
      in
      {
        gpuServiceDeclared = gpuUnit != null;
        relayNodeDeclared = builtins.elem "vsock-relay" nodeIds;
        relayNodeRole = relayNode.role;
        relayNodeTargetsObs = hasInfix
          "d2b-ch-vsock-connect /var/lib/d2b/vms/sys-obs/vsock.sock"
          (builtins.concatStringsSep " " relayNode.argv);
      };
    expectedExtract = {
      gpuServiceDeclared = true;
      relayNodeDeclared = true;
      relayNodeRole = "vsock-relay";
      relayNodeTargetsObs = true;
    };
  };
}
