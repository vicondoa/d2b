{ config, lib, pkgs, ... }:

let
  cfg = config.nixling;
  # nixling-owned access helpers (see lib.nix).
  nl = import ./lib.nix { inherit lib pkgs; };
  normalNixosVms = nl.normalNixosVms cfg.vms;
  qemuMediaVms = nl.qemuMediaVms cfg.vms;
  usbipEnvNames = lib.sort lib.lessThan (lib.unique (lib.concatMap
    (vm: lib.optional (cfg.site.yubikey.enable && vm.enable && vm.usbip.yubikey && vm.env != null) vm.env)
    (lib.attrValues cfg.vms)));
  obsOtlpPort = 14317;
  serviceControllers = [ "cpu" "memory" "pids" ];

  privateEtc = source: {
    inherit source;
    mode = "0640";
    user = "root";
    group = if cfg.daemonExperimental.enable then "nixlingd" else "root";
  };

  # (Option B from live-deploy 010 checkpoint)
  # Hash-derived ephemeral UID per principal. The matching
  # named system users (`nixling-<vm>-{gpu,snd,swtpm,runner}`)
  # are declared in `nixos-modules/host-users.nix` with the
  # SAME hash via the shared helper, so when the broker
  # `setuid`s the spawned role to this UID, NSS resolves it
  # back to the named user with its supplementary groups
  # (audio, kvm, nixling-<vm>-runner) and the per-VM ACL
  # grants the audio/graphics host modules install on
  # PipeWire / Wayland / `/dev/kvm` sockets all apply
  # transparently.
  #
  # Pure-ephemeral principals (no corresponding system user)
  # still get a unique UID — they're served by the
  # `nixlingRoleUidAcls` activation script in
  # `host-activation.nix` that walks the bundle and grants
  # per-VM-dir traversal for every distinct role UID.
  #
  # Formula moved to nixos-modules/lib.nix as the canonical
  # definition. This
  # file imports it here to keep call-site readability;
  # changing the algorithm now happens in ONE place.
  inherit (nl) stablePrincipalId;

  defaultNamespaces = {
    ipc = true;
    mount = true;
    net = false;
    pid = false;
    user = false;
    uts = false;
  };

  mkWritablePath = path: purpose: { inherit path purpose; };

  mkProfile =
    {
      profileId,
      role,
      principal,
      capabilities ? [ ],
      namespaces ? defaultNamespaces,
      seccompPolicyRef ? null,
      readOnlyPaths ? [ ],
      writablePaths ? [ ],
      deviceBinds ? [ ],
      bindMounts ? [ ],
      nixStoreReadOnly ? true,
      hideDeviceNodesByDefault ? true,
      cgroupSubtree,
      controllers ? [ ],
      delegated ? false,
      requiresStartRoot ? false,
      exceptionRef ? null,
      adr_carve_out ? null,
      # file-creation mask the broker installs in the
      # spawned child before execve. None inherits the broker's
      # umask (current behavior). Roles binding shared Unix sockets
      # (vhost-user-sound, crosvm-gpu, swtpm) declare 0o007 so the
      # bound socket has mode 0660 — combined with the per-VM
      # runtime default ACL, cloud-hypervisor's named-user entry
      # then becomes effective (mask:rw instead of mask:---).
      umask ? null,
      # (ADR 0021): when non-null, broker pre-establishes
      # a per-runner user NS and writes uid_map/gid_map. The
      # child runs fake-root inside; host-side capabilities should
      # be empty. Used by virtiofsd roles for least-privilege FS
      # serving without CAP_DAC_* on the host.
      #
      # Shape: { hostUidForZero, hostGidForZero }. Single-entry
      # mapping (in-NS UID 0 → host UID hostUidForZero).
      userNamespace ? null,
      uid ? stablePrincipalId principal,
      gid ? stablePrincipalId principal,
    }:
    let
      effectiveNamespaces =
        if userNamespace != null
        then namespaces // { user = true; }
        else namespaces;
    in
    {
      inherit
        profileId
        role
        principal
        capabilities
        seccompPolicyRef
        requiresStartRoot
        exceptionRef
        adr_carve_out
        ;
      namespaces = effectiveNamespaces;
      inherit uid gid;
      mountPolicy = {
        inherit
          readOnlyPaths
          writablePaths
          deviceBinds
          bindMounts
          nixStoreReadOnly
          hideDeviceNodesByDefault
          ;
      };
      cgroupPlacement = {
        inherit controllers delegated;
        subtree = cgroupSubtree;
      };
      userNamespace = userNamespace;
      umask = umask;
    };

  toRoleProfile = profile: {
    inherit (profile)
      profileId
      uid
      gid
      namespaces
      seccompPolicyRef
      mountPolicy
      cgroupPlacement
      ;
    adr_carve_out = profile.adr_carve_out;
    caps = profile.capabilities;
    # (ADR 0021): pass userNamespace through to the
    # processes.json RoleProfile so the broker can pre-create
    # the user NS when spawning the runner.
    userNamespace = profile.userNamespace or null;
    # pass umask through to the RoleProfile so the
    # broker can install it in the spawned child before execve.
    umask = profile.umask or null;
  };

  profileIdFor = name: nodeId: "vm-${name}-${nodeId}";
  stateDirOf = name: "${toString cfg.store.stateDir}/${name}";
  runtimeDirOf = name: "/run/nixling/${name}";
  audioRuntimeDirOf = name: "/run/nixling/vms/${name}";
  videoRuntimeDirOf = name: "/run/nixling-video/${name}";
  gpuRuntimeDirOf = name: "/run/nixling-gpu/${name}";
  # TPM socket consolidated under /run/nixling/vms/<vm>/
  # (alongside snd.sock, gpu.sock). No separate /run/swtpm/ dir.
  # Reuses the per-VM default ACL machinery in host-activation.nix.
  swtpmRuntimeDirOf = name: "/run/nixling/vms/${name}";
  vsockSocketForPort = socketPath: port: "${socketPath}_${toString port}";

  # The Gpu profile cross-domain Wayland
  # BindMount needs the operator's wayland-user numeric uid. Lazy
  # only forced for VMs with graphics.enable = true; the assertions
  # module guarantees `waylandUser` is non-null in that case.
  waylandUid =
    if cfg.site.waylandUser != null
    then toString (config.users.users.${cfg.site.waylandUser}.uid or 0)
    else "0";
  # Host primary compositor socket basename (e.g. wayland-0, or
  # wayland-1 under niri). The bind-mount src below is the host path
  # the broker grants the sidecar uid an ACL on; it MUST point at the
  # operator's real socket. See nixling.site.waylandDisplay.
  waylandHostSock = "/run/user/${waylandUid}/${cfg.site.waylandDisplay}";

  vmProfiles = name: vm:
    let
      manifest = cfg.manifest.${name};
      virtiofsdRootException = "ADR 0021 v1.1.1fu14 virtiofsd fake-root via broker pre-established user NS";
      virtiofsShares = lib.filter
        (share: (share.proto or "virtiofs") == "virtiofs")
        (nl.vmRunner config name).shares;
      virtiofsProfiles = lib.listToAttrs (lib.forEach virtiofsShares (share:
        let
          shareTag = builtins.unsafeDiscardStringContext share.tag;
          shareNodeId = "virtiofsd-${shareTag}";
          principal =
            if shareTag == "nl-gctl"
            then "nixling-${name}-gctlfs"
            else "nixling-${name}-runner";
        in {
          name = profileIdFor name shareNodeId;
          value = mkProfile {
            profileId = profileIdFor name shareNodeId;
            role = "virtiofsd";
            inherit principal;
            # (ADR 0021): with broker-pre-NS, virtiofsd
            # runs fake-root INSIDE its own user namespace. All
            # caps within the NS scope are available implicitly;
            # the host-side capabilities set is EMPTY. This is the
            # principle-of-least-privilege model: no CAP_DAC_*,
            # no CAP_SETUID, no CAP_SYS_ADMIN on the host.
            capabilities = [ ];
            seccompPolicyRef = "w1-virtiofsd";
            readOnlyPaths = [ "/nix/store" ]
              ++ lib.optional (shareTag == "nl-gctl") share.source;
            writablePaths =
              if shareTag == "nl-gctl" then [
                (mkWritablePath "${audioRuntimeDirOf name}/guest-control" "Expose the guest-control token virtiofs socket.")
              ] else [
                (mkWritablePath (stateDirOf name) "Materialize virtiofs sockets and VM-local store state.")
                (mkWritablePath (runtimeDirOf name) "Expose broker-prepared virtiofs runtime sockets.")
              ];
            cgroupSubtree = "nixling.slice/${name}/${shareNodeId}";
            controllers = serviceControllers;
            # (ADR 0021): broker pre-creates a user NS
            # mapping in-NS UID 0 → the principal's stable
            # ephemeral UID on the host. virtiofsd then runs
            # fake-root inside the NS, so it can open/serve files
            # with correct mode/UID semantics and `--sandbox=chroot`
            # works without host CAP_SYS_ADMIN.
            userNamespace = {
              hostUidForZero = stablePrincipalId principal;
              hostGidForZero = stablePrincipalId principal;
            };
            requiresStartRoot = false;
            exceptionRef = virtiofsdRootException;
          };
        }));
    in
    {
      "${profileIdFor name "host-reconcile"}" = mkProfile {
        profileId = profileIdFor name "host-reconcile";
        role = "host-reconcile";
        principal = "nixlingd";
        seccompPolicyRef = "w1-host-reconcile";
        writablePaths = [
          (mkWritablePath (stateDirOf name) "Prepare the VM state directory before process startup.")
          (mkWritablePath "/run/nixling" "Prepare daemon-owned runtime sockets and transient state.")
        ];
        cgroupSubtree = "nixling.slice/${name}/host-reconcile";
      };

      "${profileIdFor name "store-virtiofs-preflight"}" = mkProfile {
        profileId = profileIdFor name "store-virtiofs-preflight";
        role = "store-virtiofs-preflight";
        principal = "nixlingd";
        seccompPolicyRef = "w1-store-virtiofs-preflight";
        readOnlyPaths = [
          (stateDirOf name)
          "${stateDirOf name}/store-view"
          "${stateDirOf name}/store-view/live"
          "${stateDirOf name}/store-meta"
          "/nix/store"
        ];
        cgroupSubtree = "nixling.slice/${name}/store-virtiofs-preflight";
      };
    }
    // virtiofsProfiles
    // {
      "${profileIdFor name "cloud-hypervisor"}" = mkProfile {
        profileId = profileIdFor name "cloud-hypervisor";
        role = "cloud-hypervisor-runner";
        principal = "nixling-${name}-runner";
        # D4a (bounding-set drop): CAP_NET_ADMIN is
        # only required when CH opens /dev/net/tun itself and
        # calls TUNSETIFF to attach to the persistent TAP
        # (persistent-tap mode, fallback). In the default
        # tap-fd mode (site.ch.netHandoffMode = "tap-fd"),
        # the broker's CreateTapFd op opens /dev/net/tun +
        # calls TUNSETIFF pre-spawn and passes the resulting
        # TAP fd to CH via SCM_RIGHTS. CH uses fd=<N> in its
        # --net argument and requires NO CAP_NET_ADMIN.
        #
        # Enforcement: by not granting CAP_NET_ADMIN in the
        # minijail capabilities list, the kernel strips it from
        # the bounding set before execve. CH can never acquire
        # it post-spawn. The live-smoke probe asserts CapEff bit 12 == 0
        # after 10 s uptime as a
        # regression guard. In persistent-tap mode, CAP_NET_ADMIN
        # is retained (CH must call TUNSETIFF itself).
        capabilities = lib.optionals
          (cfg.site.ch.netHandoffMode == "persistent-tap")
          [ "CAP_NET_ADMIN" ];
        seccompPolicyRef = "w1-cloud-hypervisor-runner";
        readOnlyPaths = [ "/nix/store" ];
        writablePaths = [
          (mkWritablePath (stateDirOf name) "Own the VM API socket, disks, and other runtime artifacts.")
        ];
        # bind-mount /dev/kvm (and graphics-VM device
        # nodes) into the runner mount namespace. CH opens /dev/kvm
        # itself; without the bind it sees EROFS/ENOENT.
        # /dev/net/tun bind required in persistent-tap
        # mode so CH can open the device and call TUNSETIFF to
        # attach to the pre-created persistent TAP. In tap-fd mode
        # the broker pre-opens /dev/net/tun (broker runs as root,
        # outside the minijail sandbox), so CH never needs the
        # device node and the bind is omitted. /dev/vhost-net is
        # always bound — CH opens it directly for accelerated
        # virtio networking regardless of tap-handoff mode.
        #
        # The /dev/net/tun bind (persistent-tap only) exposure
        # surface is bounded by
        # (a) the broker's `CreatePersistentTap` op runs FIRST in
        #     the host-prep DAG so the named TAP always exists by
        #     the time CH attaches.
        # (b) the declarative `DeviceClass::NetTun` ioctl allowlist
        #     (packages/nixling-host/src/ioctl_policy.rs) is
        #     tightened to [TUNSETIFF, TUNSETGROUP] — the broker is
        #     the only legitimate caller of TUNSETPERSIST/TUNSETOWNER
        #     and bypasses the per-role policy via raw libc::ioctl.
        # (c) seccomp BPF compilation from the
        #     declarative ioctl matrix is wired. load_runner_seccomp
        #     compiles BPF from packages/nixling-host/src/seccomp.rs
        #     for every internal seccomp_policy_ref (including
        #     "w1-cloud-hypervisor-runner") at spawn time and the
        #     broker child closure installs it via
        #     SECCOMP_SET_MODE_FILTER BEFORE execve. The previous
        #     "Ok(None) silent-skip" deferral is retired
        #     (live_handlers.rs:1543-1563). The declarative
        #     allowlist is now an enforced runtime constraint, not
        #     documentation-only. Combined with (a) + (b) and the
        #     D4a cap-drop in tap-fd mode, post-init compromise of
        #     CH cannot escalate to additional TAP creation.
        deviceBinds = [
          "/dev/kvm"
          "/dev/vhost-net"
        ] ++ lib.optional
          (cfg.site.ch.netHandoffMode == "persistent-tap")
          "/dev/net/tun";
        cgroupSubtree = "nixling.slice/${name}/cloud-hypervisor";
        controllers = serviceControllers;
      };

      "${profileIdFor name "guest-control-health"}" = mkProfile {
        profileId = profileIdFor name "guest-control-health";
        role = "guest-control-health";
        principal = "nixlingd";
        seccompPolicyRef = "w1-guest-control-health";
        cgroupSubtree = "nixling.slice/${name}/guest-control-health";
      };
    }
    // lib.optionalAttrs vm.tpm.enable {
      # Swtpm + SwtpmFlush minijail profiles.
      #
      # The capability set is EMPTY — mkProfile
      # defaults `capabilities = [ ]`; do NOT add a `capabilities`
      # override below. CRITICAL SUBSYSTEM (AGENTS.md): the writable
      # paths declared here are a stable RW bind of
      # /var/lib/nixling/vms/<vm>/swtpm into the jail (NOT tmpfs),
      # preserving TPM 2.0 NVRAM + EK seed across daemon restarts.
      # Regression guards: tests/minijail-validator-swtpm.sh and
      # tests/integration/live/swtpm-persistence-smoke.sh. Breaking this contract
      # forces Entra/Intune re-enrollment for work-aad and similar
      # TPM-bound IdP joins.
      "${profileIdFor name "swtpm-flush"}" = mkProfile {
        profileId = profileIdFor name "swtpm-flush";
        role = "swtpm-pre-start-flush";
        principal = "nixling-${name}-swtpm";
        seccompPolicyRef = "w1-swtpm";
        writablePaths = [
          (mkWritablePath "${stateDirOf name}/swtpm" "Persist swtpm state and flush stale volatile sessions before boot.")
          (mkWritablePath (swtpmRuntimeDirOf name) "Reach the swtpm control socket during the pre-start flush.")
        ];
        cgroupSubtree = "nixling.slice/${name}/swtpm-flush";
        controllers = serviceControllers;
      };

      "${profileIdFor name "swtpm"}" = mkProfile {
        profileId = profileIdFor name "swtpm";
        role = "swtpm";
        principal = "nixling-${name}-swtpm";
        seccompPolicyRef = "w1-swtpm";
        writablePaths = [
          (mkWritablePath "${stateDirOf name}/swtpm" "Persist swtpm state for the long-lived TPM sidecar.")
          (mkWritablePath (swtpmRuntimeDirOf name) "Create the swtpm control socket for the VM.")
        ];
        cgroupSubtree = "nixling.slice/${name}/swtpm";
        controllers = serviceControllers;
        # bind swtpm control socket with mode 0660 (via
        # explicit --ctrl mode= in argv) AND umask 0o007 here so any
        # ancillary files swtpm creates inside its state dir also
        # respect the group-rw default. Combined with the per-VM
        # runtime dir default ACL (granting cloud-hypervisor's uid
        # rwx), this lets CH connect to /run/swtpm/<vm>/sock without
        # operator intervention.
        umask = 7;
        # (ADR 0021) Broker pre-creates a user NS
        # mapping in-NS UID 0 → the swtpm principal's stable
        # ephemeral UID on the host. swtpm then runs fake-root
        # inside the NS with zero host capabilities. Direct
        # translation of the virtiofsd broker-pre-NS model (ADR 0021).
        # swtpm has zero device binds + zero host caps + Unix socket
        # only — the smallest surface of all sidecars.
        userNamespace = {
          hostUidForZero = stablePrincipalId "nixling-${name}-swtpm";
          hostGidForZero = stablePrincipalId "nixling-${name}-swtpm";
        };
      };
    }
    // lib.optionalAttrs vm.graphics.enable {
      "${profileIdFor name "gpu"}" = mkProfile {
        profileId = profileIdFor name "gpu";
        role = "gpu";
        principal = "nixling-${name}-gpu";
        # Caps stay EMPTY (the original
        # matrix carried CAP_SYS_NICE; the per-role smoke proves no
        # NICE is needed at runtime — virgl/venus/cross-domain run
        # under SCHED_OTHER on this host's NVIDIA Quadro T1000).
        capabilities = [ ];
        seccompPolicyRef = "w1-gpu";
        # crosvm gpu sidecar binds the vhost-user socket
        # at /run/nixling/vms/<vm>/gpu.sock. umask 0o007 makes the
        # socket mode 0660; the per-VM runtime dir default ACL then
        # grants cloud-hypervisor rw on it via the named-user entry.
        umask = 7;
        writablePaths = [
          (mkWritablePath (stateDirOf name) "Own per-VM graphics runtime artifacts alongside the runner.")
          (mkWritablePath (gpuRuntimeDirOf name) "Expose the bound Wayland socket and GPU runtime state.")
        ];
        # Closed-set device bind set the
        # broker opens on behalf of the Gpu runner. Matches the
        # nixling_host::devices::DeviceClass taxonomy (Kvm, Dri,
        # NvidiaCtl, NvidiaRender → /dev/nvidia0 [corrected from the
        # bogus /dev/nvidia-render path], NvidiaUvm, Udmabuf).
        deviceBinds = [
          "/dev/kvm"
          "/dev/dri/renderD128"
          "/dev/nvidiactl"
          "/dev/nvidia0"
          "/dev/nvidia-uvm"
          "/dev/udmabuf"
        ];
        bindMounts = [ ];
        cgroupSubtree = "nixling.slice/${name}/gpu";
        controllers = serviceControllers;
      };

      "${profileIdFor name "video"}" = mkProfile {
        profileId = profileIdFor name "video";
        role = "video";
        principal = "nixling-${name}-video";
        # Video runs with an EMPTY capability bounding set
        # (mkProfile default; listed explicitly here so future readers don't
        # have to chase the helper).
        capabilities = [ ];
        seccompPolicyRef = "w1-video";
        deviceBinds = [
          # Render node for virtio-media decode (virtio_id=48, 2x256 queues, 256 MiB SHM region). This is the
          # default video device allowlist.
          "/dev/dri/renderD128"
        ] ++ lib.optionals (vm.graphics.videoNvidiaDecode or false) [
          # Explicit NVIDIA VA-API/NVDEC opt-in. These are the only extra
          # device nodes the proprietary nvidia-vaapi-driver opens on this
          # path; /dev remains masked before exec.
          "/dev/nvidiactl"
          "/dev/nvidia0"
          "/dev/nvidia-uvm"
        ];
        namespaces = defaultNamespaces // { pid = true; };
        writablePaths = [
          (mkWritablePath (videoRuntimeDirOf name) "Create the vhost-user video decoder socket.")
        ];
        umask = 7;
        cgroupSubtree = "nixling.slice/${name}/video";
        controllers = serviceControllers;
      };
    }
    // lib.optionalAttrs (vm.graphics.enable && vm.graphics.renderNodeOnly) {
      # (ADR 0021) Render-node-only broker-pre-NS GPU sidecar profile.
      #
      # RENDER-NODE ONLY constraint (architectural, not a v1.3 deferral)
      # This profile intentionally omits /dev/nvidiactl, /dev/nvidia0,
      # /dev/nvidia-uvm, and /dev/udmabuf. Those are root:video-owned
      # character devices; inside a single-entry user NS the host devices
      # appear with UID 65534 (overflow) and in-NS UID 0 lacks DAC access.
      # Operators requiring NVIDIA or non-render-node device passthrough
      # MUST use the legacy `gpu` profile (graphics.renderNodeOnly = false).
      #
      # SCM_RIGHTS / fd-passing justification
      # Render nodes (/dev/dri/renderD128) bypass DRM master authentication
      # entirely — DRM_IOCTL_SET_MASTER and DRM_IOCTL_AUTH_MAGIC are NOT
      # required. The broker pre-opens the fd in the parent process (before
      # clone3(CLONE_NEWUSER)), dup2's it to RENDER_NODE_INHERITED_FD (10)
      # in the child, and passes /proc/self/fd/10 as --gpu-device-node.
      # The fd survives the user-NS pivot without losing access semantics
      # because the kernel checks permissions at open time only.
      #
      # No deviceBinds: the render node fd is pre-opened and passed via
      # fd inheritance (SCM_RIGHTS into the user-NS child), not bind-mounted.
      # Mount actions are skipped for user-NS spawns (ADR 0021).
      "${profileIdFor name "gpu-render-node"}" = mkProfile {
        profileId = profileIdFor name "gpu-render-node";
        role = "gpu-render-node";
        principal = "nixling-${name}-gpu";
        # Zero host caps: the user-NS provides in-NS CAP_* without host exposure.
        capabilities = [ ];
        seccompPolicyRef = "w1-gpu-render-node";
        # umask 0o007 so the vhost-user socket created by crosvm
        # has mode 0660; the per-VM runtime dir default ACL then grants
        # cloud-hypervisor rw via the named-user entry.
        umask = 7;
        writablePaths = [
          (mkWritablePath (stateDirOf name) "Own per-VM graphics runtime artifacts alongside the runner.")
          (mkWritablePath (gpuRuntimeDirOf name) "Expose the bound Wayland socket and GPU runtime state.")
        ];
        # deviceBinds is intentionally empty: /dev/dri/renderD128 is
        # pre-opened by the broker parent and passed to the user-NS child
        # via fd inheritance (RENDER_NODE_INHERITED_FD = 10 protocol
        # constant in nixling-priv-broker/src/sys.rs). No bind-mount.
        deviceBinds = [ ];
        # No real host compositor bind-mount: the GPU runner connects to
        # the per-VM filter socket at /run/nixling-wlproxy/<vm>/wayland-0.
        # The wayland-proxy profile holds the real compositor bind-mount.
        bindMounts = [ ];
        cgroupSubtree = "nixling.slice/${name}/gpu";
        controllers = serviceControllers;
        # (ADR 0021) Broker pre-creates a user NS mapping
        # in-NS UID/GID 0 → the gpu principal's stable ephemeral UID.
        # crosvm device gpu then runs fake-root inside the NS with zero
        # host capabilities and a pre-opened render node fd.
        userNamespace = {
          hostUidForZero = stablePrincipalId "nixling-${name}-gpu";
          hostGidForZero = stablePrincipalId "nixling-${name}-gpu";
        };
      };
    }
    // lib.optionalAttrs vm.graphics.enable {
      # Wayland filter proxy role profile.
      #
      # Per ADR 0025: the host-jailed filter proxy sits between the crosvm
      # GPU sidecar and the real host compositor socket. It runs as a
      # dedicated `nixling-<vm>-wlproxy` principal with:
      #   - empty host capabilities (mandatory);
      #   - mandatory seccompPolicyRef (w1-wayland-proxy) — the proxy
      #     parses untrusted guest Wayland bytes while holding the host
      #     compositor socket so a null seccomp policy is rejected fail-closed
      #     in the broker SpawnRunner handler;
      #   - no PipeWire/Pulse socket access;
      #   - dedicated per-VM runtime dir /run/nixling-wlproxy/<vm>;
      #   - host compositor socket bind-mounted read/write at a fixed
      #     in-jail upstream path (/run/nixling-wlproxy/<vm>/upstream);
      #   - no device binds (pure AF_UNIX proxy);
      #   - explicit RLIMIT_NOFILE headroom for many guest clients and
      #     fd-bearing Wayland messages (set in argv by Wave 2/Lane A).
      #
      # ADR 0021 user namespace pattern is NOT used here: the proxy
      # binds a listen socket that other processes (including crosvm)
      # connect to, and user-NS fake-root is unnecessary for AF_UNIX
      # socket creation. The dedicated non-root host UID with no
      # capabilities is sufficient (matching the video sidecar posture).
      "${profileIdFor name "wayland-proxy"}" = mkProfile {
        profileId = profileIdFor name "wayland-proxy";
        role = "wayland-proxy";
        principal = "nixling-${name}-wlproxy";
        capabilities = [ ];
        seccompPolicyRef = "w1-wayland-proxy";
        writablePaths = [
          (mkWritablePath "/run/nixling-wlproxy/${name}"
            "Create the per-VM filter listen socket and write runtime state.")
        ];
        # The proxy connects directly to the real host compositor socket path.
        # Host activation grants this principal access to exactly that socket;
        # do not bind-mount a second socket path here.
        bindMounts = [ ];
        deviceBinds = [ ];
        cgroupSubtree = "nixling.slice/${name}/wayland-proxy";
        controllers = serviceControllers;
        # umask 0o007 so the filter listen socket has mode 0660;
        # the per-VM runtime dir default ACL then grants crosvm's
        # named-user entry rw via the GPU UID.
        umask = 7;
      };
    }
    // lib.optionalAttrs vm.audio.enable {
      "${profileIdFor name "audio"}" = mkProfile {
        profileId = profileIdFor name "audio";
        role = "audio";
        principal = "nixling-${name}-snd";
        # vhost-device-sound's libpipewire client opens
        # AF_NETLINK(NETLINK_KOBJECT_UEVENT) during pw_context_new
        # (spa-alsa-monitor) for backend probe. In a user-NS-only spawn,
        # ns_capable(net->user_ns, CAP_NET_RAW) checks the initial user NS
        # (the new net NS is owned by the initial user NS, not the process's
        # new user NS) — bind would fail with EPERM.
        #
        # Tier 1 (PIPEWIRE config elimination: PIPEWIRE_LATENCY, PIPEWIRE_NODE
        # and similar env vars) investigated and rejected: the AF_NETLINK open
        # is structural in libpipewire's context-init path (spa-alsa-monitor)
        # and precedes any user-facing configuration.
        #
        # Tier 2 resolution: combine CLONE_NEWUSER (clone3) with
        # unshare(CLONE_NEWNET) executed inside the user NS. The resulting
        # net NS is owned by the new user NS; ns_capable(net->user_ns,
        # CAP_NET_RAW) then succeeds against the new user NS. No changes to
        # RunnerIsolationSpec or sys.rs are required — NamespaceSet.net = true
        # feeds the existing unshare_namespace_flags path (CLONE_NEWNET).
        # vhost-device-sound's PipeWire + vhost-user sockets are AF_UNIX and
        # are unaffected by net NS isolation.
        namespaces = defaultNamespaces // { net = true; };
        # Closed-set seccomp profile
        # declared by name; the policy body lives in the seccomp policy
        # store and is keyed by this ref. Renamed from the v0 placeholder
        # "w1-audio-sidecar" to the canonical "w1-audio" name.
        seccompPolicyRef = "w1-audio";
        # The Wayland user's runtime dir
        # holds the PipeWire socket. libpipewire connects to
        # it, which on a read-only bind-mount fails with EROFS
        # (the socket file is in a write-mediated dir). Make
        # /run/user/<waylandUid> writable so connect succeeds.
        # The PipeWire socket file is still ACL-grant'd
        # individually by host-activation.nix's
        # nixlingRoleUidAcls script — this writablePaths just
        # ensures the mount-namespace doesn't drop it to RO.
        readOnlyPaths = [ ];
        writablePaths = [
          (mkWritablePath "${stateDirOf name}/state" "Read and persist the VM audio grant state.")
          (mkWritablePath (audioRuntimeDirOf name) "Create the PipeWire-backed vhost-user audio socket at /run/nixling/vms/<vm>/snd.sock.")
          (mkWritablePath "/run/user/${waylandUid}" "Connect to the Wayland user's PipeWire socket.")
        ];
        cgroupSubtree = "nixling.slice/${name}/audio";
        controllers = serviceControllers;
        # vhost-device-sound binds the socket at
        # /run/nixling/vms/<vm>/snd.sock. umask 0o007 makes it
        # mode 0660; the per-VM runtime dir default ACL then
        # makes cloud-hypervisor's named-user entry effective.
        umask = 7;
        # (ADR 0021) Broker pre-creates a single-entry
        # user NS mapping in-NS UID/GID 0 → the snd principal's stable
        # ephemeral UID. Combined with namespaces.net = true (above),
        # the sidecar runs fake-root inside the user-NS-owned net NS
        # with zero host capabilities. Direct translation of the
        # virtiofsd/swtpm/gpu-render-node ADR 0021 broker-pre-NS model.
        userNamespace = {
          hostUidForZero = stablePrincipalId "nixling-${name}-snd";
          hostGidForZero = stablePrincipalId "nixling-${name}-snd";
        };
      };
    }
    // lib.optionalAttrs vm.observability.enable {
      # VsockRelay role profile.
      #
      # Caps: empty. The earlier
      # matrix listed CAP_NET_RAW; corrected to empty because the
      # relay operates on pre-opened fds the broker passes in via
      # SCM_RIGHTS, so no AF_VSOCK socket call (and thus no caps)
      # are required in-role. See docs/reference/privileges.md
      # for the "pre-opened fds only" contract.
      #
      # seccompPolicyRef = "w1-vsock-relay" — must deny socket(AF_VSOCK)
      # and ptrace; tests/minijail-validator-vsock-relay.sh asserts
      # both invariants on the live host.
      "${profileIdFor name "vsock-relay"}" = mkProfile {
        profileId = profileIdFor name "vsock-relay";
        role = "vsock-relay";
        principal = "nixling-otel-relay-${name}";
        capabilities = [ ];
        seccompPolicyRef = "w1-vsock-relay";
        writablePaths = [
          (mkWritablePath (stateDirOf name) "Create the per-VM OTLP relay socket under the workload state dir.")
          (mkWritablePath "${toString cfg.store.stateDir}/${cfg.observability.vmName}" "Reach the observability VM vsock endpoint for relay forwarding.")
        ];
        cgroupSubtree = "nixling.slice/${name}/vsock-relay";
        controllers = serviceControllers;
      };
    }
    // lib.optionalAttrs (vm.usbip.yubikey && manifest.sshUser != null && manifest.staticIp != null && manifest.usbipdHostIp != null) {
      "${profileIdFor name "usbip"}" = mkProfile {
        profileId = profileIdFor name "usbip";
        role = "usbip";
        principal = "nixlingd";
        # kernel-r2-4 corrected per-role cap matrix: Usbip = CAP_NET_RAW
        # only (raw socket needed by the usbipd proxy bind on the host
        # side; the per-busid sysfs bind/unbind is sysfs-write only and
        # runs under the broker's privileged step, not under this
        # profile).
        capabilities = [ "CAP_NET_RAW" ];
        seccompPolicyRef = "w1-usbip";
        cgroupSubtree = "nixling.slice/${name}/usbip";
        controllers = serviceControllers;
      };
    };

  profileTable = lib.foldl' lib.recursiveUpdate { } (lib.mapAttrsToList vmProfiles normalNixosVms);

  qemuMediaProfiles = name: _vm: {
    "${profileIdFor name "host-reconcile"}" = mkProfile {
      profileId = profileIdFor name "host-reconcile";
      role = "host-reconcile";
      principal = "nixlingd";
      seccompPolicyRef = "w1-host-reconcile";
      writablePaths = [
        (mkWritablePath (stateDirOf name) "Prepare the qemu-media state directory before process startup.")
        (mkWritablePath "/run/nixling" "Prepare daemon-owned runtime sockets and transient state.")
      ];
      cgroupSubtree = "nixling.slice/${name}/host-reconcile";
    };

    "${profileIdFor name "wayland-proxy"}" = mkProfile {
      profileId = profileIdFor name "wayland-proxy";
      role = "wayland-proxy";
      principal = "nixling-${name}-wlproxy";
      capabilities = [ ];
      seccompPolicyRef = "w1-wayland-proxy";
      writablePaths = [
        (mkWritablePath "/run/nixling-wlproxy/${name}"
          "Create the per-VM qemu-media Wayland filter listen socket.")
      ];
      bindMounts = [ ];
      deviceBinds = [ ];
      cgroupSubtree = "nixling.slice/${name}/wayland-proxy";
      controllers = serviceControllers;
      umask = 7;
    };

    "${profileIdFor name "qemu-media"}" = mkProfile {
      profileId = profileIdFor name "qemu-media";
      role = "qemu-media-runner";
      principal = "nixling-${name}-qemu-media";
      capabilities = [ ];
      namespaces = defaultNamespaces // { pid = true; };
      seccompPolicyRef = "w1-qemu-media";
      readOnlyPaths = [ "/" ];
      writablePaths = [
        (mkWritablePath "/run/nixling/vms/${name}" "Create the QMP control socket without exposing media paths.")
        (mkWritablePath "/run/nixling-wlproxy/${name}" "Connect to the per-VM Wayland filter proxy socket.")
        (mkWritablePath (stateDirOf name) "Write only qemu-media runner state under this VM's state directory.")
      ];
      deviceBinds = [ "/dev/kvm" ];
      bindMounts = [ ];
      cgroupSubtree = "nixling.slice/${name}/qemu-media";
      controllers = serviceControllers;
    };
  };

  qemuMediaProfileTable =
    lib.foldl' lib.recursiveUpdate { } (lib.mapAttrsToList qemuMediaProfiles qemuMediaVms);

  usbipdProfilesForEnv = envName:
    let
      vmId = "sys-${envName}-usbipd";
    in {
      "${profileIdFor vmId "backend"}" = mkProfile {
        profileId = profileIdFor vmId "backend";
        role = "usbip";
        principal = "root";
        uid = 0;
        gid = 0;
        adr_carve_out = "USBIP backend usbipd requires host-root to write usbip_sockfd; broker masks host secret paths and /dev, then rebinds only the locked USB device node.";
        capabilities = [ "CAP_NET_RAW" ];
        namespaces = defaultNamespaces // { pid = true; };
        seccompPolicyRef = "w1-usbip";
        cgroupSubtree = "nixling.slice/${vmId}/backend";
        controllers = serviceControllers;
      };
      "${profileIdFor vmId "proxy"}" = mkProfile {
        profileId = profileIdFor vmId "proxy";
        role = "usbip";
        principal = "nixling-${vmId}-proxy";
        capabilities = [ ];
        seccompPolicyRef = "w1-usbip-proxy";
        cgroupSubtree = "nixling.slice/${vmId}/proxy";
        controllers = serviceControllers;
      };
    };

  usbipdProfiles =
    lib.foldl' lib.recursiveUpdate { } (map usbipdProfilesForEnv usbipEnvNames);

  # Host-scoped OTel host-bridge
  # profile. Replaces the singleton
  # `nixling-otel-host-bridge.service` (singleton scheduled for
  # removal in `nixos-modules/components/observability/host.nix`).
  # The role runs under `RunnerRole::OtelHostBridge` and receives
  # pre-opened vsock fds from the broker via SCM_RIGHTS; the
  # in-jail profile MUST NOT permit AF_VSOCK / AF_UNIX socket
  # creation (kernel-r2-4 closed-set + seccomp policy
  # `w1-otel-host-bridge`). Caps: empty per plan kernel-r2-4
  # matrix. Bind set: nixling OTel runtime dir (RW for host-egress.sock
  # listen), the obs VM's CH vsock UDS dir (RW for the textual
  # CONNECT handshake). No `/dev` binds.
  obsCfg = config.nixling.observability;
  otelHostBridgeProfile = lib.optionalAttrs (cfg.observability.enable or false) {
    "host-otel-host-bridge" = mkProfile {
      profileId = "host-otel-host-bridge";
      role = "otel-host-bridge";
      principal = "nixling-otel-bridge";
      capabilities = [ ];
      seccompPolicyRef = "w1-otel-host-bridge";
      writablePaths = [
        (mkWritablePath "/run/nixling/otel" "Host OTel runtime dir; the bridge binds host-egress.sock here for host → vsock OTLP forwarding.")
        (mkWritablePath "${toString cfg.store.stateDir}/${obsCfg.vmName}" "Reach the obs VM base CH vsock UDS for the textual CONNECT handshake into the obs VM's OTLP listener.")
      ];
      cgroupSubtree = "nixling.slice/host/otel-host-bridge";
      controllers = serviceControllers;
    };
  };

  hostProfiles = otelHostBridgeProfile;

  fullProfileTable = profileTable // qemuMediaProfileTable // usbipdProfiles // hostProfiles;

  renderedProfiles = lib.mapAttrs
    (profileId: data:
      let
        file = pkgs.writeText "nixling-${profileId}.json" (builtins.toJSON data);
      in {
        inherit data;
        path = file;
        relativePath = "minijail-profiles/${profileId}.json";
        classification = "contractPrivateNonSecret";
        sensitivity = "nonSecret";
        roleProfile = toRoleProfile data;
      })
    fullProfileTable;

  # Detect stablePrincipalId collisions at eval time.
  # stablePrincipalId = 50000 + first-24
  # bits of sha256(principal) — that's only 16.7M slots, so two
  # principals hashing to the same UID is improbable but possible
  # (birthday bound on ~5000 principals is ~99% safe; on ~12000
  # it's ~50%). Without an eval-time check, a UID collision
  # silently breaks broker-pre-NS user_namespace mapping (two
  # roles share the same host_uid_for_zero, so one role's
  # container UID 0 maps to another role's host identity).
  # Walk every principal in fullProfileTable, group by UID, and
  # fail eval if any UID has more than one distinct principal.
  principalUidPairs = lib.flatten (lib.mapAttrsToList
    (profileId: data: [
      { principal = data.principal; uid = data.uid; profileId = profileId; }
    ])
    fullProfileTable);
  principalUidByUid = lib.foldl'
    (acc: pair:
      let
        key = toString pair.uid;
        existing = acc.${key} or [ ];
      in
      acc // { ${key} = existing ++ [ pair ]; })
    { }
    principalUidPairs;
  uidCollisions = lib.filter
    (kv:
      let
        distinct = lib.unique (map (p: p.principal) kv.value);
      in
      lib.length distinct > 1)
    (lib.mapAttrsToList (uid: pairs: { inherit uid; value = pairs; }) principalUidByUid);
in
{
  config = {
    nixling._bundle.minijailProfiles = renderedProfiles;
    environment.etc = lib.mapAttrs'
      (_: profile: lib.nameValuePair "nixling/${profile.relativePath}" (privateEtc profile.path))
      renderedProfiles;

    assertions = map
      (kv: {
        assertion = false;
        message = ''
          v1.1.2 stablePrincipalId collision: UID ${kv.uid} is claimed by
          multiple distinct principals: ${lib.concatStringsSep ", "
            (lib.unique (map (p: "'${p.principal}' (profile ${p.profileId})") kv.value))}.

          stablePrincipalId is sha256(principal)[0..24] + 50000. Two
          principals hashing to the same UID is a deployment hazard
          for ADR 0021 broker-pre-NS user_namespace mapping (two
          roles would share host_uid_for_zero, so one role's
          container UID 0 maps to another role's host identity,
          breaking least-privilege isolation).

          Mitigation options (choose ONE; both require a coordinated
          rebuild + restart):

          1. Rename a colliding VM in `nixling.vms.<name>`. The
             generated principal names embed the VM name (e.g.
             `nixling-<name>-runner`, `nixling-<name>-gpu`), so
             changing the VM name moves its principal off the
             colliding hash. **THIS DOES CHANGE THE VM'S ON-DISK
             STATE PATHS** (`/var/lib/nixling/vms/<name>/` and
             every per-role subdir). Operators MUST drain the VM,
             rename the state dir to the new name, and let the
             daemon re-link the hardlink farm on next start. This
             is bigger than a config rename — plan accordingly.

          2. Rename a colliding host-singleton principal (e.g.
             `nixling-otel-bridge`). These principals live only in
             `nixos-modules/minijail-profiles.nix` and do NOT have
             on-disk state paths keyed on the principal name; the
             rename is purely an in-source rebuild. PREFERRED if a
             host-singleton is the collision source.

          For a VM-vs-VM collision, option 1 is the only path; for
          a VM-vs-host-singleton or host-vs-host collision, option
          2 is much less disruptive.

          See docs/adr/0021-broker-user-namespace-for-virtiofsd.md
          § "Stable principal IDs and collision resistance".
        '';
      })
      uidCollisions;
  };
}
