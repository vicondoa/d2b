# Nix-unit coverage for the ordinary Guest ComponentSession package shape.
#
# The package inputs are deliberately inert derivations. This case checks the
# mode-bound systemd contract without selecting a provider-specific enrollment
# implementation that U7/U8 still own.
{ mkGuestEval, lib, pkgs, flakeRoot, ... }:

let
  d2bd = pkgs.runCommand "d2bd-guest-component-session-test" { } ''
    mkdir -p "$out/bin"
    touch "$out/bin/d2bd"
  '';
  overrideD2bd = pkgs.runCommand "d2bd-guest-component-session-override-test" { } ''
    mkdir -p "$out/bin"
    touch "$out/bin/d2bd"
  '';
  broker = pkgs.runCommand "d2b-broker-guest-component-session-test" { } ''
    mkdir -p "$out/bin"
    touch "$out/bin/d2b-broker"
  '';
  optionSinks = { lib, ... }: {
    options.assertions = lib.mkOption {
      type = lib.types.listOf lib.types.anything;
      default = [ ];
    };
    options.environment.systemPackages = lib.mkOption {
      type = lib.types.listOf lib.types.package;
      default = [ ];
    };
    options.environment.etc = lib.mkOption {
      type = lib.types.attrsOf lib.types.anything;
      default = { };
    };
    options.systemd.services = lib.mkOption {
      type = lib.types.attrsOf lib.types.anything;
      default = { };
    };
    options.systemd.tmpfiles.rules = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
    };
    options.users.groups = lib.mkOption {
      type = lib.types.attrsOf lib.types.anything;
      default = { };
    };
  };
  # The Guest image's own Zone deployment graph, as an inert stand-in. This
  # case asserts the delivery - the image materializes the store object it is
  # handed at exactly the path `d2bd guest` reads - and not what is inside the
  # object. It cannot assert the document itself: this surface evaluates in an
  # isolated source root that carries no `packages/` tree, and the constructor
  # reads the per-crate `registrations.json` declarations out of it.
  #
  # The producer/verifier agreement - that `deployment-bootstrap.nix`'s own
  # `documentFor` output decodes under the daemon's verifier and names the
  # implementations this build compiles - is owned by the Rust contract tests
  # that read the rendered bytes: `//packages/d2b-core:deployment_bootstrap_contract`
  # and the guest boot path cases in `packages/d2bd/src/composition.rs`.
  deploymentGraph = pkgs.writeText "d2b-guest-deployment-bootstrap-work.json"
    "{\"graphDigest\":\"sha256:3e0db4853b89f2126441dc58f15e3255cae072e2f7e66466afb06fb62966c787\",\"zone\":\"work\"}";
  # The deployment-root-relative name the constructor produces and the module
  # delivers under, read from that constructor rather than spelled here.
  bootstrapFileName = (import (flakeRoot
    + "/nixos-modules/deployment-bootstrap.nix") { inherit lib; }).fileName;
  withGraph = (mkGuestEval {
    modules = [
      optionSinks
      componentSessionModule
      {
        d2b.componentSession = {
          enable = true;
          guestConfigPath = null;
          zone = "work";
          deploymentBootstrap = deploymentGraph;
        };
      }
    ];
    specialArgs = {
      d2bInputs = { };
      d2bHostTools = hostTools;
      name = "guest";
    };
  }).config;
  hostTools = {
    inherit d2bd broker;
  };
  hostToolOverrideKeys = [
    "d2b"
    "d2bd"
    "broker"
    "activationHelper"
    "hostActivationHelper"
    "unsafeLocalHelper"
    "resourceCompiler"
    "waylandProxy"
  ];
  d2bHostToolOverrides = lib.genAttrs hostToolOverrideKeys (_: overrideD2bd);
  componentSessionModule = import (flakeRoot + "/nixos-modules/component-session.nix");
  evaluated = (mkGuestEval {
    modules = [
      optionSinks
      componentSessionModule
      ({ ... }: {
        d2b.componentSession = {
          enable = true;
          guestConfigPath = null;
        };
      })
    ];
    specialArgs = {
      d2bInputs = { };
      d2bHostTools = hostTools;
      name = "guest";
    };
  }).config;
  overridden = (mkGuestEval {
    modules = [
      optionSinks
      componentSessionModule
      {
        d2b.componentSession = {
          enable = true;
          guestConfigPath = null;
        };
      }
    ];
    specialArgs = {
      d2bInputs = { };
      d2bHostTools = hostTools;
      inherit d2bHostToolOverrides;
      name = "guest";
    };
  }).config;
  service = evaluated.systemd.services.d2bd-guest.serviceConfig;
  packagePaths = map toString evaluated.environment.systemPackages;
in
{
  "guest-component-session/starts-d2bd-guest" = {
    expr = lib.hasInfix "/bin/d2bd guest " service.ExecStart;
    expected = true;
  };

  "guest-component-session/host-tool-override-selects-guest-daemon" = {
    expr =
      let
        packages = overridden.environment.systemPackages;
        selected = lib.findFirst
          (package: package.outPath == overrideD2bd.outPath)
          null
          packages;
        service = overridden.systemd.services.d2bd-guest.serviceConfig;
      in
      selected != null
      && selected.outPath != d2bd.outPath
      && lib.hasPrefix "${overrideD2bd.outPath}/bin/d2bd guest " service.ExecStart;
    expected = true;
  };

  "guest-component-session/uses-guest-broker-and-no-public-socket" = {
    expr = {
      broker = lib.hasInfix "--broker-socket /run/d2b/guest-broker.sock" service.ExecStart;
      public = lib.hasInfix "public.sock" service.ExecStart;
      localZone = lib.hasInfix "--config" service.ExecStart;
    };
    expected = {
      broker = true;
      public = false;
      localZone = false;
    };
  };

  "guest-component-session/binds-enrollment-inputs-at-start" = {
    expr = {
      guest = lib.hasInfix "--guest-ref Guest/guest" service.ExecStart;
      zone = lib.hasInfix "--zone local" service.ExecStart;
      schema = lib.hasInfix "--schema-fingerprint sha256:" service.ExecStart;
      privateKey = lib.hasInfix "--local-private-key /var/lib/d2b/component-session/guest.key"
        service.ExecStart;
      parentKey = lib.hasInfix "--parent-public-key /var/lib/d2b/component-session/parent.pub"
        service.ExecStart;
      writableBootId = lib.hasInfix "--boot-id-path" service.ExecStart;
    };
    expected = {
      guest = true;
      zone = true;
      schema = true;
      privateKey = true;
      parentKey = true;
      writableBootId = false;
    };
  };

  "guest-component-session/does-not-install-retired-guest-agent" = {
    expr = {
      package = lib.any (path: lib.hasInfix "d2b-guestd" path) packagePaths;
      service = builtins.hasAttr "d2b-guestd" evaluated.systemd.services;
      credential = builtins.hasAttr "LoadCredential" service;
    };
    expected = {
      package = false;
      service = false;
      credential = false;
    };
  };

  "guest-component-session/materializes-the-handed-graph-at-the-daemons-read-path" = {
    expr =
      let
        key = "d2b/deployment/${bootstrapFileName}";
      in {
        # The Guest daemon reads its graph from one fixed deployment root;
        # the image materializes the store object the image was given at
        # exactly that path, and the path is the one the constructor names,
        # so what the daemon reads is what the image closure carries.
        root = "${withGraph.d2b.componentSession.deploymentRoot}/${bootstrapFileName}";
        source = toString (withGraph.environment.etc.${key}.source or "");
        # The delivered path is derived from the constructor's own file name
        # on both sides, so a constructor that renamed its document fails
        # here instead of leaving the Guest reading a file nobody writes.
        producerFileName = bootstrapFileName;
        # An image configured with no graph delivers none: the Guest then has
        # nothing to verify and refuses to serve.
        deliveredWithoutGraph = builtins.hasAttr key evaluated.environment.etc;
      };
    expected = {
      root = "/etc/d2b/deployment/deployment-bootstrap.json";
      source = "${deploymentGraph}";
      producerFileName = "deployment-bootstrap.json";
      deliveredWithoutGraph = false;
    };
  };
}
