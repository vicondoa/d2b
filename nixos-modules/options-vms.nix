# nixling.vms.<vm>.* — per-VM submodule schema. Includes the
# component toggles (graphics.enable / tpm.enable / usbip.* /
# audio.* / audit.*) whose matching files under
# `nixos-modules/components/`
# are conditionally imported by host.nix on this submodule's
# resolved values. Extracted from options.nix for reviewability.
{ lib, ... }:

let
  qemuMediaRefType = lib.types.strMatching "^[a-z][a-z0-9-]{0,62}$";

  qemuMediaSourceType = lib.types.submodule {
    freeformType = null;
    options = {
      ref = lib.mkOption {
        type = lib.types.nullOr qemuMediaRefType;
        default = null;
        example = "installer-usb";
        description = ''
          Opaque media reference for `physical-usb` sources. Raw physical USB
          identity (busid, by-id names, serials, devnums, and block-device
          paths) is enrolled at runtime by the privileged broker and stored
          outside the Nix store. `image-file` sources do not require enrollment
          and may omit this field.
        '';
      };

      path = lib.mkOption {
        type = lib.types.nullOr (lib.types.strMatching "^/.*$");
        default = null;
        example = "/var/lib/nixling/images/installer.img";
        description = ''
          Absolute host path for a direct `image-file` source. The path is
          operator-authored configuration and may appear in Nix-store-backed
          bundle artifacts; the broker still validates the opened file at
          runtime (regular raw file, safe owner/mode/parents, no symlink escape,
          no mounted/loop-backed use, and non-blocking lease/lock checks).
          `physical-usb` sources must leave this unset and use `ref` plus
          `nixling usb enroll`.
        '';
      };

      kind = lib.mkOption {
        type = lib.types.enum [ "physical-usb" "image-file" ];
        default = "physical-usb";
        description = ''
          Source class for the opaque media reference. `physical-usb` resolves
          through root-only enrollment state. `image-file` is configured
          directly with an absolute `path` and never uses the USB enrollment
          registry.
        '';
      };

      format = lib.mkOption {
        type = lib.types.enum [ "raw" "qcow2" "iso" ];
        default = "raw";
        description = ''
          Explicit QEMU block format hint for the runner. The unsafe QEMU
          format-probing default is never used. Direct `image-file` sources are
          constrained to `raw` by eval assertions and runtime broker validation.
        '';
      };

      readOnly = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = ''
          Whether the media source should be attached read-only when the QEMU
          media runner lands. Defaults to true for removable/installer media.
        '';
      };
    };
  };

  qemuMediaSlotType = lib.types.submodule ({ name, ... }: {
    freeformType = null;
    options = {
      source = lib.mkOption {
        type = lib.types.nullOr qemuMediaSourceType;
        default = null;
        description = ''
          Optional media source currently inserted in removable slot `${name}`.
          The source submodule is closed: declarative USB bus-id fields such as
          `busid`, `busids`, `busId`, and `busIds` are rejected by the schema.
        '';
      };

      readOnly = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Default read-only policy for media inserted into this slot.";
      };
    };
  });
in
{
  options.nixling.vms = lib.mkOption {
    description = "MicroVMs to declare via the nixling framework.";
    default = { };
    type = lib.types.attrsOf (lib.types.submodule ({ name, ... }: {
      # options-vms-removed.nix's mkRemovedOptionModule shim
      # was retired here. It cannot be imported into an `attrsOf
      # submodule` per-instance because the per-VM submodule layer
      # does not have an `assertions` option (NixOS assertions live
      # at the top-level config root). The defense-in-depth
      # assertion in `assertions.nix` is the sole supervisor-removal
      # error path; the friendly message text matches the original
      # mkRemovedOptionModule wording verbatim.
      options = {
        enable = lib.mkOption {
          type = lib.types.bool;
          default = true;
          description = "Whether this microVM is registered with microvm.nix.";
        };
        runtime.kind = lib.mkOption {
          type = lib.types.enum [ "nixos" "qemu-media" ];
          default = "nixos";
          description = ''
            Runtime family for this VM declaration. `nixos` (the default)
            uses nixling's per-VM NixOS evaluator and Cloud Hypervisor DAG.
            `qemu-media` is the foundational external-media VM kind; it is
            manual-only for now, emits a paused fd-backed QEMU runner, and
            deliberately skips the per-VM NixOS evaluator.
          '';
        };

        qemuMedia = lib.mkOption {
          type = lib.types.submodule {
            freeformType = null;
            options = {
              source = lib.mkOption {
                type = lib.types.nullOr qemuMediaSourceType;
                default = null;
                description = ''
                  Optional primary media source for `runtime.kind =
                  "qemu-media"`. This closed submodule rejects undeclared
                  declarative USB bus-id fields instead of ignoring them.
                '';
              };

              removableSlots = lib.mkOption {
                type = lib.types.attrsOf qemuMediaSlotType;
                default = { };
                description = ''
                  Named removable-media slots for the future QEMU media
                  runtime. Each slot and each nested slot source is a closed
                  submodule, so unsupported declarative USB bus-id fields are
                  schema errors.
                '';
              };

              window = lib.mkOption {
                type = lib.types.submodule {
                  freeformType = null;
                  options = {
                    niriBorderColor = lib.mkOption {
                      type = lib.types.nullOr (lib.types.strMatching "^#[0-9a-fA-F]{6}$");
                      default = null;
                      example = "#800080";
                      description = ''
                        Active border color for this qemu-media VM's host QEMU
                        window in the niri compositor, as a six-digit CSS hex
                        color (`#rrggbb`). Used only when
                        `nixling.site.niriVmBorders.enable = true`.

                        Set to `null` (the default) to use the deterministic
                        palette color derived from the VM name. When set, the
                        value is placed verbatim into the generated KDL
                        `active-color` field for the host QEMU window rule.
                      '';
                    };
                  };
                };
                default = { };
                description = ''
                  Host-window presentation options for `runtime.kind =
                  "qemu-media"` declarations.
                '';
              };
            };
          };
          default = { };
          description = ''
            Initial external QEMU media runtime schema. The process contract
            emits a safe paused QEMU baseline; media fd handoff and hotplug
            handling use opaque runtime requests instead of declarative USB
            identities.
          '';
        };
        autostart = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = ''
            Start this microVM at host boot. Graphics VMs cannot
            autostart (the systemd unit has no Wayland session).
            Net VMs declared via nixling.envs.<env> default
            to autostart = true (set in network.nix).
          '';
        };
        config = lib.mkOption {
          # `deferredModule` lets multiple definitions merge as
          # modules: framework-side `lib.mkDefault { imports = [...]; }`
          # combines with consumer-side `{ users.users.x = ...; }`
          # via the module system's natural attribute merge,
          # instead of one definition stomping the other (which is
          # what `types.unspecified` used to do).
          type = lib.types.deferredModule;
          default = { };
          example = lib.literalExpression "{ imports = [ ../../vms/foo.nix ]; }";
          description = ''
            NixOS module merged into the guest's configuration.
            Typically `{ imports = [ ../../vms/<name>.nix ]; }`.

            Multiple definitions (e.g. one from the framework, one
            from a consumer override) are merged as modules: imports
            are concatenated, attribute paths recursively combined.
          '';
        };

        guestConfigFile = lib.mkOption {
          type = lib.types.nullOr lib.types.path;
          default = null;
          example = lib.literalExpression "./vms/work.guest.nix";
          description = ''
            Path to a dedicated **guest-editable** NixOS module holding
            this VM's in-guest OS layer — the software installed and run
            inside the VM (`environment.systemPackages`, `services.*`,
            in-guest `users.users.*`, `programs.*`, files, desktop).

            It is merged into the guest's configuration like `config`,
            but is **contained**: it may set only guest OS options, and
            is rejected at eval time (a hard assertion) if it sets any
            host-owned `microvm.*` (runner substrate: mounts, devices,
            volumes, hypervisor args, kernel, vsock, …) or `nixling.*`
            (framework) option. Those host-owned concerns stay in the
            host-owned `config` above, which the guest cannot edit.

            This is the surface that the in-VM config-sync workflow
            (`nixling config sync` / `diff` / `approve`) edits: an
            operator can change it from inside the VM, sync the change
            back to the host, review it, and approve it — without ever
            being able to escape the VM's own OS boundary.
          '';
        };

        # Nixling-owned per-VM evaluator output.
        # Populated by host.nix's composeVm pass which runs the
        # consumer's `config` through the nixling-owned per-VM
        # NixOS evaluator (see vm-evaluator.nix). Stored at the
        # SIBLING attribute `nixling._computed.vms.<name>` rather
        # than `nixling.vms.<name>.computed` to avoid the
        # NixOS-module infinite-recursion that occurs when a
        # mapAttrs-over-cfg.vms write target is the same attribute
        # path the iteration reads from.
        # (Retained as an internal placeholder option for
        # compat with consumers that may already touch the path;
        # always empty at the option level.)
        computed = lib.mkOption {
          type = lib.types.unspecified;
          default = { config = { }; options = { }; };
          internal = true;
          description = ''
            DEPRECATED in v1.1: per-VM evaluation output moved to
            `nixling._computed.vms.<name>` to avoid module-system
            infinite recursion. Read via `nl.vmRunner config name`
            from `nixos-modules/lib.nix` (helpers route to the new
            location automatically).
          '';
        };

        graphics.enable = lib.mkEnableOption ''
          virtio-gpu + Wayland cross-domain forward to the host
          compositor. Implies hypervisor = cloud-hypervisor, pulls
          in the spectrum-os-patched CH plus the patched crosvm GPU
          sidecar, and auto-launches a foot terminal in the guest
          so the VM has a visible window the moment it boots.
        '';

        graphics.crossDomainTrusted = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = ''
            Gate the cross-domain Wayland context type in the crosvm GPU
            sidecar. Default false. Set true only for VMs where cross-domain
            forwarding is the primary use case (e.g. a Wayland-forwarding
          launchpad VM running FreeRDP or another remote-desktop client).
            Must be false for any VM running Docker (privileged-container escape
            inside the VM could attack the host compositor via cross-domain).
          '';
        };

        graphics.xwayland.enable = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = ''
            Whether the guest Wayland proxy should expose an X11 socket
            and spawn Xwayland for legacy clients. Disable for VMs whose
            workload must be strictly Wayland-only. Default false because
            Xwayland is a legacy compatibility surface and not every host
            compositor supports it.
          '';
        };

        graphics.videoSidecar = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = ''
            Spawn the per-VM crosvm video-decoder sidecar for the
            H264 virtio-media decode path. Requires graphics.enable
            and uses nixling's patched Cloud Hypervisor
            --vhost-user-media support plus the patched crosvm
            video-decoder build. Default false so graphics VMs boot
            without the media backend unless explicitly opted in.
          '';
        };

        graphics.videoNvidiaDecode = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = ''
            Extend the video sidecar's closed device allowlist for the
            NVIDIA VA-API/NVDEC backend. Requires
            graphics.videoSidecar = true. When enabled, the broker masks
            /dev and exposes only /dev/dri/renderD128 plus the NVIDIA
            character devices required by nvidia-vaapi-driver.
          '';
        };

        graphics.virglVideo = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = ''
            Experimental: enable virglrenderer/rutabaga video forwarding
            (`VIRGL_RENDERER_USE_VIDEO`) on the crosvm GPU sidecar. This is
            the path Firefox's VA-API decoder would need, but it is
            default-off because earlier testing deadlocked the GPU command
            loop when the guest advertised video decode capabilities.
          '';
        };

        graphics.renderNodeOnly = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = ''
            When
            true, the gpu sidecar runs inside a single-entry user NS
            with zero host caps; the broker pre-opens
            /dev/dri/renderD128 and passes the fd via SCM_RIGHTS.

            REQUIRES: render-node-only configuration (no NVIDIA,
            no /dev/udmabuf). NVIDIA / non-render-node device
            passthrough must use the default (false) value to keep
            the legacy gpu profile.

            Rationale: render nodes bypass DRM master authentication
            (no DRM_IOCTL_SET_MASTER / DRM_IOCTL_AUTH_MAGIC required),
            so the fd survives user-NS pivot without losing access
            semantics. Other device classes (NVIDIA, udmabuf) need
            direct DAC access to host device nodes which a
            single-entry user-NS cannot grant (host UID 0 appears as
            UID 65534 inside the NS).

            Default false to preserve v1.1.x compat for NVIDIA users.
          '';
        };

        graphics.waylandFilter.enable = lib.mkOption {
          type = lib.types.bool;
          default = true;
          description = ''
            Enable the host-jailed Wayland filter proxy between the crosvm
            GPU sidecar and the real host compositor. When true (the default),
            crosvm connects to the per-VM filter socket at
            `/run/nixling-wlproxy/<vm>/wayland-0`; when false, the
            `wayland-proxy` DAG node is not emitted and the GPU runner uses
            the legacy direct compositor socket path. Has no effect unless
            `graphics.crossDomainTrusted = true`.
          '';
        };

        graphics.waylandFilter.debugLogging = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = ''
            Enable verbose wl-proxy protocol tracing for this VM's host-side
            Wayland filter runner. This sets `WL_PROXY_DEBUG=1` and a
            VM-specific `WL_PROXY_PREFIX`, causing raw Wayland protocol
            messages to be emitted to the runner's stderr and therefore the
            broker/journald log stream.

            This is intended only for short-lived debugging. Protocol traces
            can include application metadata such as titles, app IDs, registry
            names, object IDs, and file-descriptor numbers.
          '';
        };

        graphics.waylandFilter.byteLogging = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = ''
            Enable raw Wayland transport hexdump logging for this VM's
            host-side Wayland filter runner. This sets `WL_PROXY_HEXDUMP=1`
            and logs bounded recv/send byte prefixes plus fd counts for
            short-lived debugging of protocol corruption.

            Output may contain application metadata and protocol payload
            bytes. Do not leave enabled during normal operation.
          '';
        };

        graphics.waylandFilter.denyGlobals = lib.mkOption {
          type = lib.types.listOf lib.types.str;
          default = [ ];
          example = [ "wp_drm_lease_device_v1" ];
          description = ''
            Additional Wayland globals to deny beyond the secure defaults.
            Each entry is an interface name (e.g. `wp_drm_lease_device_v1`).
            Repeated `--deny-global` arguments are passed to the filter proxy.
            The filter proxy emits runtime advisory diagnostics if an entry
            shadows a nixling-required or high-risk rule.
          '';
        };

        graphics.waylandFilter.allowGlobals = lib.mkOption {
          type = lib.types.listOf lib.types.str;
          default = [ ];
          example = [ "zwp_linux_dmabuf_v1" ];
          description = ''
            Wayland globals to explicitly allow even if denied by the secure
            defaults. Each entry is passed as `--allow-global` to the filter
            proxy. The filter proxy emits runtime advisory diagnostics when
            used; the operator is explicitly narrowing the security boundary.
          '';
        };

        graphics.waylandFilter.maxVersions = lib.mkOption {
          type = lib.types.attrsOf lib.types.ints.positive;
          default = { };
          example = { xdg_wm_base = 3; };
          description = ''
            Maximum advertised Wayland protocol versions for specific globals.
            Each entry maps an interface name to a version cap and is passed
            as `--max-version INTERFACE=VERSION` to the filter proxy. This is
            a compatibility override; unlike allow/deny global overrides, it
            does not currently emit a runtime policy warning.
          '';
        };

        graphics.waylandFilter.dmabufAllow = lib.mkOption {
          type = lib.types.listOf lib.types.str;
          default = [ ];
          example = [ "XR24:linear" "AR24:0x0100000000000001" ];
          description = ''
            dmabuf format/modifier filters to allow unconditionally, in
            `FORMAT[:MODIFIER]` form. `FORMAT` may be `all`, a fourcc such as
            `XR24`, a hexadecimal value, or a decimal value. `MODIFIER` may be
            `linear`, `invalid`, a hexadecimal value, or a decimal value.

            These rules are applied by the host Wayland filter to legacy
            `zwp_linux_dmabuf_v1.modifier` events and v4/v5 feedback tranche
            format tables. Allow rules override deny rules.
          '';
        };

        graphics.waylandFilter.dmabufDeny = lib.mkOption {
          type = lib.types.listOf lib.types.str;
          default = [ ];
          example = [ "all:linear" ];
          description = ''
            dmabuf format/modifier filters to deny unless explicitly allowed,
            in `FORMAT[:MODIFIER]` form. This can be used to keep dmabuf
            feedback v4/v5 available while hiding known-bad modifiers from
            guests.
          '';
        };

        graphics.niriBorderColor = lib.mkOption {
          type = lib.types.nullOr (lib.types.strMatching "^#[0-9a-fA-F]{6}$");
          default = null;
          example = "#7fc8ff";
          description = ''
            Active border color for this VM's windows in the niri
            compositor, as a six-digit CSS hex color (`#rrggbb`).
            Used only when `nixling.site.niriVmBorders.enable = true`.

            Set to `null` (the default) to use the deterministic
            palette color derived from the VM name — each VM name
            maps to a stable distinct color so the generated KDL
            works without any per-VM configuration.

            When set, must be a valid six-digit hex color starting
            with `#`.  The value is placed verbatim into the
            generated KDL `active-color` field for this VM.

            The inactive border color is always `#505050` and is not
            configurable here; configure it directly in your niri
            config if you need a different inactive color.
          '';
        };

        tpm.enable = lib.mkEnableOption ''
          swtpm 2.0-backed TPM device exposed to the guest as a TPM
          CRB at /dev/tpm0 + /dev/tpmrm0. Implies hypervisor =
          cloud-hypervisor (the only one microvm.nix can wire swtpm
          to). Persistent state lives in
          /var/lib/nixling/vms/<vm>/swtpm/ on the host.
        '';

        writableStoreOverlaySize = lib.mkOption {
          type = lib.types.ints.positive;
          default = 1073741824;
          description = ''
            Size in bytes of the writable store overlay disk image at
            <filename>/var/lib/nixling/vms/&lt;vm&gt;/store-overlay.img</filename>.
            Default is 1 GiB (1073741824 bytes). Only used when the
            guest VM sets <option>microvm.writableStoreOverlay</option>
            to a non-null path; the broker provisions the disk image via
            a <literal>DiskInit</literal> plan-op before SpawnRunner.
          '';
        };

        usbip.yubikey = lib.mkEnableOption ''
          YubiKey USBIP passthrough. Loads vhci_hcd in the guest and
          installs the usbip CLI so the `nixling usb <vm>` host-side
          wrapper can redirect a plugged-in Yubico device from the
          host's xhci to this VM via USBIP. Host-side daemon is
          per-env (`sys-<env>-usbipd`/`proxy` broker runner); see
          `nixos-modules/network.nix`.
        '';

        usbip.busids = lib.mkOption {
          type = lib.types.listOf lib.types.str;
          default = [ ];
          example = [ "1-1.4" ];
          description = ''
            Exact USBIP busids this VM is allowed to claim inside its
            env. Emitted into `host.json.environments[].usbipBusidLocks[]
            .busIds` for the daemon/broker intent resolver. Leave empty
            to preserve the legacy placeholder `pending` fallback used by
            v0.4-era host.json fixtures.
          '';
        };

        audio.enable = lib.mkEnableOption ''
          Host microphone + speaker, mediated via vhost-user-sound +
          PipeWire. Setting this only enables the *capability*: the
          per-VM state file at
          /var/lib/nixling/vms/<vm>/state/audio-state.json is what
          actually decides whether the VM has a virtio-sound device
          at any given moment. Both mic and speaker default to OFF
          on first materialisation (unless allowMicByDefault /
          allowSpeakerByDefault below are flipped). Live grant/revoke
          is via `nixling audio …`. The VM shows up in plasma-pa as a
          PipeWire client named `nixling-<vm>` for per-stream mute /
          volume.
        '';

        audio.allowMicByDefault = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = ''
            Initial value of the `mic` field when the per-VM audio
            state file is first materialised. Only consulted at file
            creation time; subsequent edits via `nixling audio …`
            persist. Default false (explicit-grant model).
          '';
        };

        audio.allowSpeakerByDefault = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = ''
            Initial value of the `speaker` field when the per-VM
            audio state file is first materialised. Only consulted at
            file creation time; subsequent edits via `nixling audio
            …` persist. Default false (explicit-grant model).
          '';
        };

        audio.users = lib.mkOption {
          type = lib.types.listOf lib.types.str;
          default = [ ];
          example = [ "alice" ];
          description = ''
            Guest-side usernames that should be added to the `audio`
            group inside the VM. The virtio-snd ALSA driver exposes
            `/dev/snd/*` with mode `0660 root:audio`, so an
            interactive user wanting to talk to WirePlumber /
            PipeWire from a non-logind-active session needs the
            group explicitly.

            Defaults to `[ ssh.user ]` when `ssh.user` is set,
            otherwise the empty list. Override here if the VM has
            additional interactive users.
          '';
        };

        audit = {
          enable = lib.mkEnableOption ''
            guest-side auditd with forwarding to the existing
            observability pipeline (guest Alloy → vsock → Loki)
          '';

          rules = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [
              "-w /etc/passwd -p wa -k identity"
              "-w /etc/shadow -p wa -k identity"
              "-w /etc/sudoers -p wa -k priv-esc"
            ];
            description = ''
              Curated guest-side audit rules. Propagated to the
              guest's `nixling.audit.rules` when `audit.enable = true`.
              The default excludes `execve` argv capture because
              command lines routinely carry secrets; add that rule
              explicitly only for short-lived, high-sensitivity audits.
            '';
          };
        };

        observability.enable = lib.mkEnableOption ''
          guest OpenTelemetry collector + reverse OTLP tunnel to the
          observability stack VM
        '';

        observability.scrapeJournal = lib.mkOption {
          type = lib.types.bool;
          default = true;
          description = ''
            Whether the guest OpenTelemetry collector follows this VM's
            systemd journal (journald receiver) and forwards it to the
            SigNoz backend as logs. Default on for observed VMs; set to
            false to suppress guest log collection.
          '';
        };

        observability.scrapeNodeMetrics = lib.mkOption {
          type = lib.types.bool;
          default = true;
          description = ''
            Whether the guest OpenTelemetry collector scrapes this VM's
            node/system metrics.
          '';
        };

        env = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          example = "work";
          description = ''
            Name of a `nixling.envs.<env>` this VM belongs to.
            When non-null, the framework auto-derives the VM's MAC
            and IP from `(env, index)`, creates a tap on the env's
            LAN bridge, and registers a dnsmasq host-reservation on
            the env's net VM. The VM's guest-side network config
            becomes pure DHCP.

            null = legacy mode (must hand-roll microvm.interfaces +
            systemd.network in the guest). Net new VMs should set
            env.
          '';
        };

        index = lib.mkOption {
          type = lib.types.ints.between 10 250;
          default = 10;
          description = ''
            Workload-VM IP index within its env's LAN subnet. The
            VM's IP = <lan-subnet-prefix>.<index>. Range 10–250 to
            leave room for the net VM (.1), gateway-ish reservations
            (.2–.9), and DHCP pool overflow (.251–.254). Must be
            unique within an env.
          '';
        };

        # DEPRECATED. Pre-env hint used by the CLI to know the VM's
        # IP. With `env` set, the framework derives the IP and the
        # CLI reads it from the same source — don't set this.
        staticIp = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          example = "10.10.0.10";
          description = ''
            DEPRECATED. Use `env` + `index` instead — those derive
            the IP and propagate it everywhere (CLI manifest,
            dnsmasq reservation, guest networkd). Setting both
            `staticIp` and `env` is an error.

            null = no static IP and no env = the CLI cannot ssh.
          '';
        };

        ssh = {
          user = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            example = "alice";
            description = "Username for `nixling`-driven SSH into the VM.";
          };
          keyPath = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            defaultText = lib.literalExpression
              "\"\${config.nixling.site.keysDir}/<vm>_ed25519\"";
            example = "~/.ssh/example-vm_ed25519";
            description = ''
              Private-key path for `nixling`-driven SSH into the VM.

              **Default (when unset): derived from
              `nixling.site.keysDir`** as
              `<keysDir>/<vm>_ed25519`, matching the framework-
              managed Ed25519 key generated by `host-keys.nix` on
              every activation. The derived default is applied at
              module-merge time in `host-keys.nix`, so the apparent
              `null` here just means "let the framework decide".

              Override only if you supply your own per-VM key (e.g.
              a hardware-backed key whose private half does not
              live in `<keysDir>/`). The framework-managed key
              itself is still generated regardless; see
              `nixling.site.keysDir`.

              Setting this to `null` AND `nixling.site.keysDir` to
              an unreadable location is the only way to opt out of
              the framework-managed key entirely — `nixling` CLI
              subcommands that need SSH (`keys rotate`, `switch`,
              …) will refuse to run when the resolved path doesn't
              exist.
            '';
          };
        };

        sudo = lib.mkEnableOption ''
          passwordless sudo for the VM's SSH user (`ssh.user`). When
          enabled, the framework adds a NOPASSWD sudoers rule for
          the user inside the guest, allowing `sudo` from the
          nixling SSH session without an interactive password prompt.
          Useful for development/debugging VMs where the SSH user
          needs root for `tpm2_flushcontext`, `systemctl restart`,
          etc.
        '';

        userAuthorizedKeys = lib.mkOption {
          type = lib.types.listOf
            (lib.types.oneOf [ lib.types.path lib.types.str ]);
          default = [ ];
          example = lib.literalExpression ''
            [ ./keys/alice_id_ed25519.pub ]
          '';
          description = ''
            Per-VM authorized SSH keys, merged with the global
            `nixling.site.userAuthorizedKeys` set. Both lists are
            handed to the VM's `nixling-load-host-keys.service`,
            which writes them — together with the framework's own
            nixling-managed pubkey for this VM — into the SSH user's
            `authorized_keys` file.

            Entries follow the same shape as
            `nixling.site.userAuthorizedKeys`: paths to `.pub` files
            or literal pubkey strings, validated at eval time.
          '';
        };

        homeManager = {
          enable = lib.mkEnableOption ''
            Home Manager inside this VM. Imports
            `nixos-modules/components/home-manager.nix` which pulls in
            the upstream HM NixOS module and applies sensible defaults
            (useGlobalPkgs, useUserPackages, .hm-backup, inputs in
            extraSpecialArgs).
          '';
          users = lib.mkOption {
            type = lib.types.attrsOf lib.types.unspecified;
            default = { };
            description = ''
              Per-user HM config. See
              `nixos-modules/components/home-manager.nix` for the
              expected shape.
            '';
          };
        };

        guest.control = {
          enable = lib.mkOption {
            type = lib.types.bool;
            default = false;
            description = ''
              Enable the guest-control credential/share surface and the
              `nixling-guestd` service wiring for this VM. The static
              guest-control binaries are installed for every VM; this option
              opts the VM into the live guest-control plane (credential share
              plus guestd), which serves the readiness Health probe, `config
              sync` reads (`ReadGuestFile`), and — when `guest.exec` is also
              enabled — admin guest exec. All of it runs over the
              authenticated guest-control vsock, not SSH.
            '';
          };
          auth.tokenFile = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            example = "/run/secrets/nixling/work/guest-control-token";
            description = ''
              Absolute runtime path to an operator-managed guest-control token
              file. Do not use Nix path literals such as `./token`; those can
              copy secret material into `/nix/store`. When null, nixling
              generates a stable per-VM fallback token under
              `nixling.site.stateDir` outside the runner-writable per-VM
              state root.

              Runtime validation requires the source and its parent
              directories to be symlink-free, the file to be regular,
              root-owned, outside `/nix/store`, and inaccessible to
              group/world permission bits. Nixling materializes a
              root-owned copy readable only by the dedicated
              `nixling-<vm>-gctlfs` guest-control virtiofsd principal.
            '';
          };
        };

        guest.exec = {
          enable = lib.mkOption {
            type = lib.types.bool;
            default = false;
            description = ''
              Enable the guest-control exec runtime for this VM.

              This wires the guestd exec service so admin operators (callers in
              `nixling.site.adminUsers`) can run `nixling vm exec` against this
              VM over the authenticated guest-control vsock — no SSH. Exec is
              off by default; enabling it requires `guest.control.enable = true`
              and a workload user (`ssh.user`).

              Every exec runs the requested command as the VM's workload user
              (`ssh.user`) — never as root — inside a real PAM login session
              (`systemd-run --property=PAMName=login --uid=<user>`), so the
              command sees the same environment an SSH login would
              (`XDG_RUNTIME_DIR`, the login-shell profile, …). Users elevate
              with `sudo` inside the session.
            '';
          };

          # Tombstones (removed options): guest-control exec now always
          # runs as the VM's workload user (`ssh.user`) in a PAM login
          # session — never root — and there is no per-VM exec user
          # allowlist. These hidden stubs keep legacy assignments landing
          # on the friendly migration assertions in assertions.nix instead
          # of a cryptic "option does not exist" module-system error.
          allowRoot = lib.mkOption {
            type = lib.types.bool;
            default = false;
            internal = true;
            visible = false;
            description = ''
              REMOVED. Guest-control exec always runs as the VM's workload
              user (`ssh.user`), never root. Elevate with `sudo` inside the
              session. See the migration assertion in assertions.nix.
            '';
          };

          users = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [ ];
            internal = true;
            visible = false;
            description = ''
              REMOVED. Guest-control exec always targets the VM's single
              workload user (`ssh.user`); there is no per-VM exec user
              allowlist. See the migration assertion in assertions.nix.
            '';
          };

          detachedMaxRuntimeSec = lib.mkOption {
            type = lib.types.ints.unsigned;
            default = 0;
            example = 86400;
            description = ''
              Default runtime ceiling, in seconds, for detached execs on this
              VM. `0` (the default) means no ceiling: a detached exec may run
              indefinitely until it exits or is cancelled.

              When non-zero, guestd passes the value to the per-exec transient
              unit as a `RuntimeMaxSec` ceiling; a detached exec exceeding it is
              terminated and reported as expired. This is a guest-enforced
              backstop, not a substitute for explicit cancellation.
            '';
          };

          interactiveMaxRuntimeSec = lib.mkOption {
            type = lib.types.ints.unsigned;
            default = 0;
            example = 28800;
            description = ''
              Default runtime ceiling, in seconds, for interactive (TTY) execs
              on this VM. `0` (the default) means no ceiling: an interactive
              session is connection-owned and may run indefinitely until it
              exits or the controlling connection drops.

              This ceiling applies only to interactive `tty = true`,
              non-detached execs. Non-interactive attached execs keep their
              fixed built-in runtime ceiling regardless of this value.
            '';
          };
        };

        # REMOVED. The submodule path is kept ONLY so that
        # legacy consumer assignments produce a readable assertion
        # error rather than the cryptic "option does not exist"
        # message the module system would emit if we simply dropped
        # the option. The stub is `internal = true` so it does not
        # appear in generated docs.
        #
        # Migration: see CHANGELOG.md and the
        # `vicondoa/entrablau.nix` flake README for the new
        # composition pattern (per-VM config.imports of the sibling
        # flake's nixosModules.default).
        entra-id = lib.mkOption {
          type = lib.types.attrsOf lib.types.unspecified;
          default = { };
          internal = true;
          visible = false;
          description = ''
            REMOVED. Use `vicondoa/entrablau.nix`'s
            module per-VM via `nixling.vms.<vm>.config.imports`.
          '';
        };
      };
    }));
  };

  # Per-VM evaluation outputs stored OUTSIDE `nixling.vms.<name>` to
  # avoid module-system infinite recursion.
  # host.nix's composeVm pass populates `nixling._computed.vms.<name>`
  # with the evaluated NixOS attrset (config + options) for each
  # enabled VM. lib.nix helpers (vmRunner / vmToplevel /
  # vmDeclaredRunner) route through this attribute.
  options.nixling._computed = lib.mkOption {
    type = lib.types.attrsOf lib.types.unspecified;
    default = { };
    internal = true;
    visible = false;
    description = ''
      Internal storage for per-VM evaluator outputs. Populated by
      host.nix's composeVm pass. Stored here (not under
      `nixling.vms.<name>.computed`) to avoid the NixOS module-
      system infinite-recursion that occurs when a mapAttrs over
      cfg.vms writes back to the same `nixling.vms` attribute it
      reads from.
    '';
  };
}
