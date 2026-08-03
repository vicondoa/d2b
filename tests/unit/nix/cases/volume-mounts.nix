# nix-unit cases migrated from tests/volume-mounts-eval.sh (group D).
#
# Asserts the shared `nixos-modules/lib.nix` volume helpers
# (volumeSerial / volumeHostPath / volumeFileSystem / volumeSizeBytes /
# volumeDiskInitEligible / volumeSerialIssues) - Cloud Hypervisor disk
# serials, guest fileSystems mounts, MiB->bytes, DiskInit eligibility, and
# the duplicate/reserved/overlong/unsafe serial issue sets.
#
# The v3 cases below also evaluate the reachable Volume ResourceType module.
# They deliberately inspect the Volume module's canonical assertion records
# rather than the aggregate system assertion list. The bundle integrity gate
# separately rejects path-shaped ResourceSpec fields by throwing; reading the
# module-local records keeps these policy vectors focused on the Volume rules.
#
# The "module callsites use the shared helpers" grep checks the bash gate
# also carried are NOT value assertions; they migrate to the hermetic
# flake.checks.<sys>.module-helper-wiring derivation (see flake.nix).
{ d2bLib, mkEval, lib, pkgs, ... }:

let
  varVolume = {
    image = "var.img";
    mountPoint = "/var";
    size = 1024;
    fsType = "ext4";
    serial = null;
  };
  externalVolume = {
    image = "/tmp/external.img";
    mountPoint = "/mnt/external";
    size = 1;
    fsType = "ext4";
  };
  nonExt4Volume = {
    image = "data.img";
    mountPoint = "/data";
    size = 1;
    fsType = "xfs";
  };
  qcowVolume = {
    image = "qcow.img";
    mountPoint = "/qcow";
    size = 1;
    fsType = "ext4";
    imageType = "qcow2";
  };

  issues = d2bLib.volumeSerialIssues [
    { image = "var.img"; }
    { image = "var.img"; }
    { image = "rootfs.img"; }
    { image = "this-name-is-definitely-too-long.img"; }
    { image = "ok.img"; serial = "bad,serial"; }
    { image = "ok2.img"; serial = "bad=serial"; }
    { image = "empty.img"; serial = ""; }
  ];

  fs = d2bLib.volumeFileSystem varVolume;

  volumeArtifact = pkgs.writeText "d2b-test-volume-provider" "volume-provider";
  volumeResource = {
    type = "Volume";
    spec = {
      providerRef = "Provider/volume-local";
      kind = "state";
      source = {
        executionRef = "Host/host-system";
        settings = {
          kind = "local-path";
          sourcePolicyId = "state-root";
        };
      };
      layout = [{
        path = "state";
        type = "directory";
        ownerRef = "User/alice";
        groupRef = "User/alice";
        mode = "0700";
        noFollow = true;
      }];
      views.controller = {
        path = "";
        rights = [ "read" "write" "traverse" ];
      };
      attachments = [ ];
    };
  };

  volumeBase = { ... }: {
    boot.loader.grub.enable = false;
    boot.loader.systemd-boot.enable = false;
    boot.initrd.includeDefaultModules = false;
    fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
    environment.etc."machine-id".text = "00000000000000000000000000000000";
    system.stateVersion = "25.11";

    d2b.artifacts.volume-local = {
      package = volumeArtifact;
      type = "provider";
    };
    d2b.zones.local-root.resources = {
      alice.type = "User";
      volume-local = {
        type = "Provider";
        spec = {
          artifactId = "volume-local";
          config = { };
        };
      };
      host-system = {
        type = "Host";
        spec.providerRef = "Provider/volume-local";
      };
      state = volumeResource;
    };
  };

  validVolume = (mkEval [ volumeBase ]).config;
  generatedGuestVolume = (mkEval [
    volumeBase
    {
      d2b.zones.local-root.resources.guest = {
        type = "Guest";
        spec.tpmEnabled = true;
      };
    }
  ]).config;
  generatedStoreView =
    generatedGuestVolume.d2b._resourceCompiler.volumeGenerated.byZone.local-root.store-view-guest;
  generatedTpm =
    generatedGuestVolume.d2b._resourceCompiler.volumeGenerated.byZone.local-root.swtpm-guest;
  attachmentVolume = (mkEval [
    volumeBase
    {
      d2b.zones.local-root.resources.state.spec.attachments = [{
        executionRef = "Guest/guest";
        transport = "virtiofs";
        view = "controller";
        access = "read-only";
        mountPath = "/state";
        settings = {
          posixAcl = false;
          xattr = false;
          cache = "auto";
          inodeFileHandles = "never";
          threadPoolSize = null;
          socketGroup = null;
        };
      }];
      d2b.zones.local-root.resources.guest = {
        type = "Guest";
        spec = { };
      };
    }
  ]).config;
  digestWithExtraUser = (mkEval [
    volumeBase
    { d2b.zones.local-root.resources.extra-user.type = "User"; }
  ]).config;
  typedShorthand = (mkEval [
    volumeBase
    {
      d2b.volumes.typed = {
        zone = "local-root";
        spec = volumeResource.spec;
      };
    }
  ]).config;

  volumeAssertionsOf = sys:
    sys.config.d2b._resourceCompiler.volumeValidation;

  failures = sys:
    lib.filter (a: !a.assertion
      && lib.hasPrefix "d2b.zones.local-root.resources.state" a.message)
      (volumeAssertionsOf sys);

  hasFailure = needle: sys:
    lib.any (a: lib.hasInfix needle a.message) (failures sys);

  invalid = override:
    mkEval [ volumeBase override ];

  blockImageBase = {
    d2b.zones.local-root.resources.state.spec = {
      kind = "durable";
      source.settings = {
        kind = "block-image";
        sourcePolicyId = "disk-root";
      };
    };
  };

  blockImageWithQuota = lib.recursiveUpdate blockImageBase {
    d2b.zones.local-root.resources.state.spec.quota = {
      maxBytes = 4096;
      enforcement = "none";
    };
  };

  blockImageAttachment = {
    d2b.zones.local-root.resources.state.spec.attachments = [{
      executionRef = "Host/host-system";
      transport = "virtiofs";
      view = "controller";
      access = "read-only";
      mountPath = "/disk";
    }];
  };

  tmpfsBase = {
    d2b.zones.local-root.resources.state.spec = {
      kind = "tmp";
      source.settings = {
        kind = "tmpfs";
      };
      quota = {
        maxBytes = 4096;
        maxInodes = 32;
        enforcement = "hard";
      };
    };
  };

  layoutEntry = changes:
    {
      path = "state";
      type = "directory";
      ownerRef = "User/alice";
      groupRef = "User/alice";
      mode = "0700";
    } // changes;
in
{
  "volume-mounts/serial-null-defaults" = {
    expr = d2bLib.volumeSerial varVolume;
    expected = "var";
  };
  "volume-mounts/serial-sanitizes-delimiters" = {
    expr = d2bLib.volumeSerial { image = "bad,name=still.img"; };
    expected = "bad-name-still";
  };
  "volume-mounts/host-path-relative" = {
    expr = d2bLib.volumeHostPath "/var/lib/d2b/vms" "work" varVolume;
    expected = "/var/lib/d2b/vms/work/var.img";
  };
  "volume-mounts/host-path-absolute" = {
    expr = d2bLib.volumeHostPath "/var/lib/d2b/vms" "work" externalVolume;
    expected = "/tmp/external.img";
  };
  "volume-mounts/fs-device" = {
    expr = fs.device;
    expected = "/dev/disk/by-id/virtio-var";
  };
  "volume-mounts/fs-fstype" = {
    expr = fs.fsType;
    expected = "ext4";
  };
  "volume-mounts/fs-needed-for-boot" = {
    expr = fs.neededForBoot;
    expected = true;
  };
  "volume-mounts/fs-options-waits-modules" = {
    expr = builtins.elem "x-systemd.after=systemd-modules-load.service" fs.options;
    expected = true;
  };
  "volume-mounts/size-bytes" = {
    expr = d2bLib.volumeSizeBytes varVolume;
    expected = 1073741824;
  };
  "volume-mounts/disk-init-relative-ext4-raw" = {
    expr = d2bLib.volumeDiskInitEligible varVolume;
    expected = true;
  };
  "volume-mounts/disk-init-absolute" = {
    expr = d2bLib.volumeDiskInitEligible externalVolume;
    expected = false;
  };
  "volume-mounts/disk-init-non-ext4" = {
    expr = d2bLib.volumeDiskInitEligible nonExt4Volume;
    expected = false;
  };
  "volume-mounts/disk-init-non-raw" = {
    expr = d2bLib.volumeDiskInitEligible qcowVolume;
    expected = false;
  };
  "volume-mounts/issues-duplicates" = {
    expr = builtins.elem "var" issues.duplicates;
    expected = true;
  };
  "volume-mounts/issues-reserved" = {
    expr = builtins.elem "rootfs" issues.reserved;
    expected = true;
  };
  "volume-mounts/issues-too-long" = {
    expr = builtins.elem "this-name-is-definitely-too-long" issues.tooLong;
    expected = true;
  };
  "volume-mounts/issues-unsafe-comma" = {
    expr = builtins.elem "bad,serial" issues.unsafe;
    expected = true;
  };
  "volume-mounts/issues-unsafe-equals" = {
    expr = builtins.elem "bad=serial" issues.unsafe;
    expected = true;
  };
  "volume-mounts/issues-unsafe-empty" = {
    expr = builtins.elem "" issues.unsafe;
    expected = true;
  };

  # --- v3 Volume ResourceType wiring and fail-closed policy vectors ---
  "volume-mounts/v3-valid-resource-reaches-canonical-bundle" = {
    expr =
      let
        resources = validVolume.d2b._bundle.zoneResourceBundlesV3.local-root.data.resources;
        state = builtins.head (lib.filter (resource:
          resource.type == "Volume" && resource.metadata.name == "state") resources);
      in state.spec.source.settings.sourcePolicyId;
    expected = "state-root";
  };
  "volume-mounts/v3-valid-volume-assertions-pass" = {
    expr = lib.all (assertion: assertion.assertion)
      (lib.filter
        (assertion:
          lib.hasPrefix "d2b.zones.local-root.resources.state" assertion.message)
        validVolume.d2b._resourceCompiler.volumeValidation);
    expected = true;
  };

  "volume-mounts/v3-valid-resource-reaches-topical-compiler" = {
    expr = validVolume.d2b._resourceCompiler.volumes.byZone.local-root.state.type;
    expected = "Volume";
  };
  "volume-mounts/v3-typed-volume-shorthand-reaches-compiler" = {
    expr = typedShorthand.d2b._resourceCompiler.volumeShorthand.local-root.typed.type;
    expected = "Volume";
  };
  "volume-mounts/v3-typed-volume-shorthand-enters-zone-bundle" = {
    expr = builtins.any
      (resource:
        resource.type == "Volume" && resource.metadata.name == "typed")
      typedShorthand.d2b._bundle.zoneResourceBundlesV3.local-root.data.resources;
    expected = true;
  };
  "volume-mounts/v3-typed-volume-shorthand-rejects-anchored-path-escape" = {
    expr = (builtins.tryEval (
      builtins.deepSeq
        ((mkEval [
          volumeBase
          {
            d2b.volumes.bad = {
              zone = "local-root";
              spec = volumeResource.spec // {
                layout = [
                  ((builtins.head volumeResource.spec.layout)
                    // { path = "/escape"; })
                ];
              };
            };
          }
        ]).config.d2b.volumes.bad.spec.layout)
        true
    )).success;
    expected = false;
  };
  "volume-mounts/v3-store-view-generates-root-state-and-gcroots" = {
    expr = map (entry: entry.path) generatedStoreView.spec.layout;
    expected = [
      ""
      "live"
      "live/.d2b-marker-guest"
      "meta"
      "meta/generations"
      "meta/current"
      "state"
      "gcroots"
      "sync.lock"
    ];
  };
  "volume-mounts/v3-store-view-gcroots-is-not-under-meta" = {
    expr = builtins.any (entry: entry.path == "meta/gcroots") generatedStoreView.spec.layout;
    expected = false;
  };
  "volume-mounts/v3-store-view-attachment-is-provider-neutral-virtiofs" = {
    expr = {
      transport = (builtins.head generatedStoreView.spec.attachments).transport;
      access = (builtins.head generatedStoreView.spec.attachments).access;
      view = (builtins.head generatedStoreView.spec.attachments).view;
      mountPath = (builtins.head generatedStoreView.spec.attachments).mountPath;
    };
    expected = {
      transport = "virtiofs";
      access = "read-only";
      view = "ro-store";
      mountPath = "/nix/.ro-store";
    };
  };
  "volume-mounts/v3-generated-volumes-enter-zone-bundle" = {
    expr = map (resource: resource.metadata.name)
      (lib.filter (resource: resource.type == "Volume")
        generatedGuestVolume.d2b._bundle.zoneResourceBundlesV3.local-root.data.resources);
    expected = [ "state" "store-view-guest" "swtpm-guest" ];
  };
  "volume-mounts/v3-tpm-volume-is-secret-adjacent-and-never-recreated" = {
    expr = {
      kind = generatedTpm.spec.kind;
      sensitivity = (builtins.head generatedTpm.spec.layout).sensitivity;
      createPolicy = (builtins.head generatedTpm.spec.layout).createPolicy;
      repairPolicy = (builtins.head generatedTpm.spec.layout).repairPolicy;
      ownerRef = (builtins.head generatedTpm.spec.layout).ownerRef;
    };
    expected = {
      kind = "state";
      sensitivity = "secret-adjacent";
      createPolicy = "create-if-never-provisioned";
      repairPolicy = "fail-closed";
      ownerRef = "User/d2b-guest-swtpm";
    };
  };
  "volume-mounts/v3-virtiofs-attachment-emits-vfd-user-and-provider" = {
    expr = {
      vfd = attachmentVolume.d2b._resourceCompiler.volumeGenerated.byZone.local-root."vol-state-vfd".type;
      provider = attachmentVolume.d2b._resourceCompiler.volumeGenerated.byZone.local-root.volume-virtiofs.spec.artifactId;
    };
    expected = {
      vfd = "User";
      provider = "volume-virtiofs-provider";
    };
  };
  "volume-mounts/v3-volume-bundle-digest-covers-all-resources" = {
    expr =
      validVolume.d2b._bundle.zoneResourceBundlesV3.local-root.data.contentHash
      != digestWithExtraUser.d2b._bundle.zoneResourceBundlesV3.local-root.data.contentHash;
    expected = true;
  };

  "volume-mounts/v3-block-image-requires-byte-quota" = {
    expr = hasFailure "quota.maxBytes is required"
      (invalid blockImageBase);
    expected = true;
  };

  "volume-mounts/v3-block-image-requires-durable-or-ephemeral-kind" = {
    expr = hasFailure "must be durable or ephemeral for a block-image source"
      (invalid (lib.recursiveUpdate blockImageBase {
        d2b.zones.local-root.resources.state.spec.kind = "state";
      }));
    expected = true;
  };

  "volume-mounts/v3-block-image-requires-virtio-blk" = {
    expr = hasFailure "must be virtio-blk for a block-image Volume"
      (invalid (lib.recursiveUpdate blockImageWithQuota blockImageAttachment));
    expected = true;
  };

  "volume-mounts/v3-tmpfs-requires-bounded-hard-quota" = {
    expr = hasFailure "quota.maxBytes and"
      (invalid {
        d2b.zones.local-root.resources.state.spec = {
          kind = "tmp";
          source.settings.kind = "tmpfs";
        };
      });
    expected = true;
  };

  "volume-mounts/v3-tmpfs-requires-hard-enforcement" = {
    expr = hasFailure "quota.enforcement must be hard"
      (invalid (lib.recursiveUpdate tmpfsBase {
        d2b.zones.local-root.resources.state.spec.quota.enforcement = "none";
      }));
    expected = true;
  };

  "volume-mounts/v3-quota-limits-must-be-positive" = {
    expr = hasFailure "quota.maxBytes must be a positive integer"
      (invalid {
        d2b.zones.local-root.resources.state.spec.quota = {
          maxBytes = 0;
          enforcement = "none";
        };
      });
    expected = true;
  };

  "volume-mounts/v3-non-symlink-must-not-follow" = {
    expr = hasFailure "noFollow must be true for every entry"
      (invalid {
        d2b.zones.local-root.resources.state.spec.layout = [
          (layoutEntry { noFollow = false; })
        ];
      });
    expected = true;
  };

  "volume-mounts/v3-symlink-must-follow-explicit-target" = {
    expr = hasFailure "noFollow must be false for a symlink"
      (invalid {
        d2b.zones.local-root.resources.state.spec.layout = [
          (layoutEntry {
            type = "symlink";
            target = "state";
            noFollow = true;
          })
        ];
      });
    expected = true;
  };

  "volume-mounts/v3-symlink-target-cannot-be-empty" = {
    expr = hasFailure "target is required for a symlink"
      (invalid {
        d2b.zones.local-root.resources.state.spec.layout = [
          (layoutEntry {
            type = "symlink";
            target = "";
            noFollow = false;
          })
        ];
      });
    expected = true;
  };

  "volume-mounts/v3-recursive-no-recursion-invariant-conflict" = {
    expr = hasFailure "recursive conflicts with no-recursive-mutation"
      (invalid {
        d2b.zones.local-root.resources.state.spec.layout = [
          (layoutEntry {
            recursive = true;
            invariants = [ "no-recursive-mutation" ];
          })
        ];
      });
    expected = true;
  };

  "volume-mounts/v3-tmpfs-create-readiness-cannot-rely-on-provisioning" = {
    expr = hasFailure "readiness cannot depend on prior provisioning"
      (invalid (lib.recursiveUpdate tmpfsBase {
        d2b.zones.local-root.resources.state.spec.layout = [
          (layoutEntry {
            createPolicy = "create-if-never-provisioned";
            restartPolicy = "recreate-after-owner-death";
          })
        ];
      }));
    expected = true;
  };

  "volume-mounts/v3-tmpfs-restart-readiness-cannot-preserve-controller-state" = {
    expr = hasFailure "readiness cannot depend on controller restart persistence"
      (invalid (lib.recursiveUpdate tmpfsBase {
        d2b.zones.local-root.resources.state.spec.layout = [
          (layoutEntry {
            createPolicy = "create-if-absent";
            restartPolicy = "preserve-across-controller-restart";
          })
        ];
      }));
    expected = true;
  };

  "volume-mounts/v3-source-policy-never-carries-host-path" = {
    expr = hasFailure "must not carry a host path"
      (invalid {
        d2b.zones.local-root.resources.state.spec.source.settings.path = "/etc";
      });
    expected = true;
  };
  "volume-mounts/v3-anchored-path-rejects-absolute-layout-path" = {
    expr = hasFailure "anchored inside the Volume"
      (invalid {
        d2b.zones.local-root.resources.state.spec.layout = [
          (layoutEntry { path = "/escape"; })
        ];
      });
    expected = true;
  };
  "volume-mounts/v3-anchored-path-rejects-parent-component" = {
    expr = hasFailure "anchored inside the Volume"
      (invalid {
        d2b.zones.local-root.resources.state.spec.layout = [
          (layoutEntry { path = "safe/../escape"; })
        ];
      });
    expected = true;
  };
  "volume-mounts/v3-symlink-target-rejects-absolute-path" = {
    expr = hasFailure "target is required for a symlink"
      (invalid {
        d2b.zones.local-root.resources.state.spec.layout = [
          (layoutEntry {
            type = "symlink";
            target = "/escape";
            noFollow = false;
          })
        ];
      });
    expected = true;
  };
  "volume-mounts/v3-layout-schema-rejects-unknown-field" = {
    expr = hasFailure "unsupported layout field"
      (invalid {
        d2b.zones.local-root.resources.state.spec.layout = [
          (layoutEntry { unexpected = true; })
        ];
      });
    expected = true;
  };

  "volume-mounts/v3-attachment-settings-are-typed" = {
    expr = hasFailure "settings.cache must be auto, always, or never"
      (invalid {
        d2b.zones.local-root.resources.state.spec.attachments = [{
          executionRef = "Host/host-system";
          transport = "virtiofs";
          view = "controller";
          access = "read-only";
          mountPath = "/state";
          settings.cache = "drop";
        }];
      });
    expected = true;
  };
}
