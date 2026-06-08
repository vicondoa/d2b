{ config, lib, pkgs, ... }:

let
  cfg = config.nixling;
  # v1.1-P8: nixling-owned access helpers (see lib.nix).
  nl = import ./lib.nix { inherit lib pkgs; };
  enabledVms = lib.filterAttrs (_: vm: vm.enable) cfg.vms;
  obsOtlpPort = 14317;
  serviceControllers = [ "cpu" "memory" "pids" ];

  privateEtc = source: {
    inherit source;
    mode = "0640";
    user = "root";
    group = if cfg.daemonExperimental.enable then "nixlingd" else "root";
  };

  # v1.1.1fu11 (Option B from live-deploy 010 checkpoint):
  # Hash-derived ephemeral UID per principal. The matching
  # named system users (`nixling-<vm>-{gpu,snd,swtpm,runner}`)
  # are declared in `nixos-modules/host-users.nix` with the
  # SAME hash via the shared helper, so when the broker
  # `setuid()`s the spawned role to this UID, NSS resolves it
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
  # v1.1.2-final-R1 (panel-software HIGH): formula moved to
  # nixos-modules/lib.nix as the canonical definition. This
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
      # v1.1.2fu36: file-creation mask the broker installs in the
      # spawned child before execve. None inherits the broker's
      # umask (current behavior). Roles binding shared Unix sockets
      # (vhost-user-sound, crosvm-gpu, swtpm) declare 0o007 so the
      # bound socket has mode 0660 — combined with the per-VM
      # runtime default ACL, cloud-hypervisor's named-user entry
      # then becomes effective (mask:rw instead of mask:---).
      umask ? null,
      # v1.1.1fu14 (ADR 0021): when non-null, broker pre-establishes
      # a per-runner user NS and writes uid_map/gid_map. The
      # child runs fake-root inside; host-side capabilities should
      # be empty. Used by virtiofsd roles for least-privilege FS
      # serving without CAP_DAC_* on the host.
      #
      # Shape: { hostUidForZero, hostGidForZero }. Single-entry
      # mapping (in-NS UID 0 → host UID hostUidForZero).
      userNamespace ? null,
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
      uid = stablePrincipalId principal;
      gid = stablePrincipalId principal;
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
    # v1.1.1fu14 (ADR 0021): pass userNamespace through to the
    # processes.json RoleProfile so the broker can pre-create
    # the user NS when spawning the runner.
    userNamespace = profile.userNamespace or null;
    # v1.1.2fu36: pass umask through to the RoleProfile so the
    # broker can install it in the spawned child before execve.
    umask = profile.umask or null;
  };

  profileIdFor = name: nodeId: "vm-${name}-${nodeId}";
  stateDirOf = name: "${toString cfg.store.stateDir}/${name}";
  runtimeDirOf = name: "/run/nixling/${name}";
  audioRuntimeDirOf = name: "/run/nixling/vms/${name}";
  videoRuntimeDirOf = name: "/run/nixling-video/${name}";
  gpuRuntimeDirOf = name: "/run/nixling-gpu/${name}";
  # v1.1.2fu36: TPM socket consolidated under /run/nixling/vms/<vm>/
  # (alongside snd.sock, gpu.sock). No separate /run/swtpm/ dir.
  # Reuses the per-VM default ACL machinery in host-activation.nix.
  swtpmRuntimeDirOf = name: "/run/nixling/vms/${name}";
  vsockSocketForPort = socketPath: port: "${socketPath}_${toString port}";

  # P1 ph1-p1-gpu-seccomp: the Gpu profile cross-domain Wayland
  # BindMount needs the operator's wayland-user numeric uid. Lazy:
  # only forced for VMs with graphics.enable = true; the assertions
  # module guarantees `waylandUser` is non-null in that case.
  waylandUid =
    if cfg.site.waylandUser != null
    then toString (config.users.users.${cfg.site.waylandUser}.uid or 0)
    else "0";

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
        in {
          name = profileIdFor name shareNodeId;
          value = mkProfile {
            profileId = profileIdFor name shareNodeId;
            role = "virtiofsd";
            principal = "nixling-${name}-runner";
            # v1.1.1fu14 (ADR 0021): with broker-pre-NS, virtiofsd
            # runs fake-root INSIDE its own user namespace. All
            # caps within the NS scope are available implicitly;
            # the host-side capabilities set is EMPTY. This is the
            # principle-of-least-privilege model: no CAP_DAC_*,
            # no CAP_SETUID, no CAP_SYS_ADMIN on the host.
            capabilities = [ ];
            seccompPolicyRef = "w1-virtiofsd";
            readOnlyPaths = [ "/nix/store" ];
            writablePaths = [
              (mkWritablePath (stateDirOf name) "Materialize virtiofs sockets and VM-local store state.")
              (mkWritablePath (runtimeDirOf name) "Expose broker-prepared virtiofs runtime sockets.")
            ];
            cgroupSubtree = "nixling.slice/${name}/${shareNodeId}";
            controllers = serviceControllers;
            # v1.1.1fu14 (ADR 0021): broker pre-creates a user NS
            # mapping in-NS UID 0 → the principal's stable
            # ephemeral UID on the host. virtiofsd then runs
            # fake-root inside the NS, so it can open/serve files
            # with correct mode/UID semantics and `--sandbox=chroot`
            # works without host CAP_SYS_ADMIN.
            userNamespace = {
              hostUidForZero = stablePrincipalId "nixling-${name}-runner";
              hostGidForZero = stablePrincipalId "nixling-${name}-runner";
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
          "${stateDirOf name}/store"
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
        # P1 per-role capability matrix (kernel-r2-4 corrected):
        # CloudHypervisor declares CAP_NET_ADMIN as the **setup-time
        # union** cap; the static minijail allowlist cannot express
        # "transient", so the runner role's startup code MUST drop
        # CAP_NET_ADMIN explicitly before entering its main loop.
        # The setup-time use is the SCM_RIGHTS tap-fd recv path; once
        # the fd is received, no further net-admin syscalls are made.
        capabilities = [ "CAP_NET_ADMIN" ];
        seccompPolicyRef = "w1-cloud-hypervisor-runner";
        readOnlyPaths = [ "/nix/store" ];
        writablePaths = [
          (mkWritablePath (stateDirOf name) "Own the VM API socket, disks, and other runtime artifacts.")
        ];
        # v1.1.1fu13e: bind-mount /dev/kvm (and graphics-VM device
        # nodes) into the runner mount namespace. CH opens /dev/kvm
        # itself; without the bind it sees EROFS/ENOENT.
        # v1.1.2fu33: /dev/net/tun must be bind-mounted so CH can
        # open and configure its TAP interfaces inside the sandbox.
        #
        # IMPORTANT (panel-networking + panel-security
        # v1.1.2-final-R2): the runner's static minijail caps grant
        # CAP_NET_ADMIN as the **setup-time union** (see
        # `capabilities` above + kernel-r2-4 comment). At spawn
        # time CH has CAP_NET_ADMIN in its effective set; this is
        # what lets the SCM_RIGHTS tap-fd recv path work and what
        # CH itself uses to call TUNSETIFF on the pre-existing
        # persistent TAP. CH's published behaviour is to drop
        # CAP_NET_ADMIN before entering its main loop (after
        # device init), but the minijail static allowlist cannot
        # express "transient" — operators MUST audit the
        # cloud-hypervisor build to confirm this drop happens.
        #
        # The /dev/net/tun bind exposure surface is bounded by:
        # (a) the broker's `CreatePersistentTap` op runs FIRST in
        #     the host-prep DAG (ApplyNftables → CreatePersistentTap
        #     → SetBridgePortFlags → OpenVhostNet → SpawnRunner(CH))
        #     so the named TAP always exists by the time CH attaches.
        # (b) the declarative `DeviceClass::NetTun` ioctl allowlist
        #     (packages/nixling-host/src/ioctl_policy.rs) is
        #     tightened to [TUNSETIFF, TUNSETGROUP] — the broker is
        #     the only legitimate caller of TUNSETPERSIST/TUNSETOWNER
        #     and bypasses the per-role policy via raw libc::ioctl.
        # (c) ADR-tracked v1.2 follow-up: seccomp BPF compilation
        #     from the declarative ioctl matrix is NOT yet wired
        #     (load_runner_seccomp returns Ok(None) for non-absolute
        #     seccomp_policy_ref values). Until that lands, the
        #     declarative allowlist is contractual only — operators
        #     of high-threat environments should treat a
        #     post-init-compromise of CH as capable of creating
        #     additional TAPs until the BPF enforcement closes the
        #     gap. See CHANGELOG.md "Known limitations".
        deviceBinds = [
          "/dev/kvm"
          "/dev/vhost-net"
          "/dev/net/tun"
        ];
        cgroupSubtree = "nixling.slice/${name}/cloud-hypervisor";
        controllers = serviceControllers;
      };

      "${profileIdFor name "guest-ssh-readiness"}" = mkProfile {
        profileId = profileIdFor name "guest-ssh-readiness";
        role = "guest-ssh-readiness";
        principal = "nixlingd";
        seccompPolicyRef = "w1-guest-ssh-readiness";
        cgroupSubtree = "nixling.slice/${name}/guest-ssh-readiness";
      };
    }
    // lib.optionalAttrs vm.tpm.enable {
      # P1 Swtpm + SwtpmFlush minijail profiles.
      #
      # Per plan kernel-r2-4 the capability set is EMPTY — mkProfile
      # defaults `capabilities = [ ]`; do NOT add a `capabilities`
      # override below. CRITICAL SUBSYSTEM (AGENTS.md): the writable
      # paths declared here are a stable RW bind of
      # /var/lib/nixling/vms/<vm>/swtpm into the jail (NOT tmpfs),
      # preserving TPM 2.0 NVRAM + EK seed across daemon restarts.
      # Regression guards: tests/minijail-validator-swtpm.sh and
      # tests/swtpm-persistence-smoke.sh. Breaking this contract
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
        # v1.1.2fu36: bind swtpm control socket with mode 0660 (via
        # explicit --ctrl mode= in argv) AND umask 0o007 here so any
        # ancillary files swtpm creates inside its state dir also
        # respect the group-rw default. Combined with the per-VM
        # runtime dir default ACL (granting cloud-hypervisor's uid
        # rwx), this lets CH connect to /run/swtpm/<vm>/sock without
        # operator intervention.
        umask = 7;
      };
    }
    // lib.optionalAttrs vm.graphics.enable {
      "${profileIdFor name "gpu"}" = mkProfile {
        profileId = profileIdFor name "gpu";
        role = "gpu";
        principal = "nixling-${name}-gpu";
        # P1 kernel-r2-4 corrected: caps stay EMPTY (the original
        # matrix carried CAP_SYS_NICE; the per-role smoke proves no
        # NICE is needed at runtime — virgl/venus/cross-domain run
        # under SCHED_OTHER on this host's NVIDIA Quadro T1000).
        capabilities = [ ];
        seccompPolicyRef = "w1-gpu";
        # v1.1.2fu36: crosvm gpu sidecar binds the vhost-user socket
        # at /run/nixling/vms/<vm>/gpu.sock. umask 0o007 makes the
        # socket mode 0660; the per-VM runtime dir default ACL then
        # grants cloud-hypervisor rw on it via the named-user entry.
        umask = 7;
        writablePaths = [
          (mkWritablePath (stateDirOf name) "Own per-VM graphics runtime artifacts alongside the runner.")
          (mkWritablePath (gpuRuntimeDirOf name) "Expose the bound Wayland socket and GPU runtime state.")
        ];
        # P1 ph1-p1-device-matrix: closed-set device bind set the
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
        # P1 cross-domain Wayland: broker mounts the host's
        # `/run/user/<waylandUser-uid>/wayland-0` socket into the
        # sidecar's mount namespace at the role-local in-sandbox
        # path so the sidecar can never traverse `/run/user/<uid>`.
        bindMounts = [
          { src = "/run/user/${waylandUid}/wayland-0";
            dst = "${gpuRuntimeDirOf name}/wayland-0"; }
        ];
        cgroupSubtree = "nixling.slice/${name}/gpu";
        controllers = serviceControllers;
      };

      "${profileIdFor name "video"}" = mkProfile {
        profileId = profileIdFor name "video";
        role = "video";
        principal = "nixling-${name}-gpu";
        # P1 kernel-r2-4: video runs with an EMPTY capability bounding set
        # (mkProfile default; listed explicitly here so future readers don't
        # have to chase the helper).
        capabilities = [ ];
        seccompPolicyRef = "w1-video";
        readOnlyPaths = [
          # Render node for virtio-media decode (kernel-8 wire contract:
          # virtio_id=48, 2x256 queues, 256 MiB SHM region). Bind RO; the
          # vhost-user-media backend only opens this for DRM ioctls, never
          # writes through it.
          "/dev/dri/renderD128"
        ];
        writablePaths = [
          (mkWritablePath (videoRuntimeDirOf name) "Create the vhost-user video decoder socket.")
        ];
        cgroupSubtree = "nixling.slice/${name}/video";
        controllers = serviceControllers;
      };
    }
    // lib.optionalAttrs vm.audio.enable {
      "${profileIdFor name "audio"}" = mkProfile {
        profileId = profileIdFor name "audio";
        role = "audio";
        principal = "nixling-${name}-snd";
        # P1 ph1-p1-cap-matrix (kernel-r2-4): Audio = CAP_NET_RAW only.
        # vhost-user-sound's libpipewire client opens AF_NETLINK for the
        # virtio-snd backend probe; CAP_NET_RAW gates that bind.
        capabilities = [ "CAP_NET_RAW" ];
        # P1 ph1-p1-closed-set-profiles: closed-set seccomp profile
        # declared by name; the policy body lives in the seccomp policy
        # store and is keyed by this ref. Renamed from the v0 placeholder
        # "w1-audio-sidecar" to the canonical "w1-audio" P1 name.
        seccompPolicyRef = "w1-audio";
        # v1.1.1fu11 (Option B): the Wayland user's runtime dir
        # holds the PipeWire socket. libpipewire connect()s to
        # it, which on a read-only bind-mount fails with EROFS
        # (the socket file is in a write-mediated dir). Make
        # /run/user/<waylandUid> writable so connect() succeeds.
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
        # v1.1.2fu36: vhost-device-sound binds the socket at
        # /run/nixling/vms/<vm>/snd.sock. umask 0o007 makes it
        # mode 0660; the per-VM runtime dir default ACL then
        # makes cloud-hypervisor's named-user entry effective.
        umask = 7;
      };
    }
    // lib.optionalAttrs vm.observability.enable {
      # P1 VsockRelay role profile.
      #
      # Caps: empty (kernel-r2-4 corrected matrix). The earlier
      # matrix listed CAP_NET_RAW; corrected to empty because the
      # relay operates on pre-opened fds the broker passes in via
      # SCM_RIGHTS, so no AF_VSOCK socket() call (and thus no caps)
      # are required in-role. See docs/reference/privileges.md
      # §"P1 role profiles" for the "pre-opened fds only" contract.
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

  profileTable = lib.foldl' lib.recursiveUpdate { } (lib.mapAttrsToList vmProfiles enabledVms);

  # P1 decision 5 + observability-4: host-scoped OTel host-bridge
  # profile. Replaces the singleton
  # `nixling-otel-host-bridge.service` (singleton scheduled for
  # P3 removal in `nixos-modules/components/observability/host.nix`).
  # The role runs under `RunnerRole::OtelHostBridge` and receives
  # pre-opened vsock fds from the broker via SCM_RIGHTS; the
  # in-jail profile MUST NOT permit AF_VSOCK / AF_UNIX socket
  # creation (kernel-r2-4 closed-set + seccomp policy
  # `w1-otel-host-bridge`). Caps: empty per plan kernel-r2-4
  # matrix. Bind set: alloy runtime dir (RW for host-egress.sock
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
        (mkWritablePath "/run/alloy" "Host alloy runtime dir; the bridge binds host-egress.sock here for alloy → vsock OTLP forwarding.")
        (mkWritablePath "${toString cfg.store.stateDir}/${obsCfg.vmName}" "Reach the obs VM base CH vsock UDS for the textual CONNECT handshake into the obs VM's OTLP listener.")
      ];
      cgroupSubtree = "nixling.slice/host/otel-host-bridge";
      controllers = serviceControllers;
    };
  };

  hostProfiles = otelHostBridgeProfile;

  fullProfileTable = profileTable // hostProfiles;

  renderedProfiles = lib.mapAttrs
    (profileId: data:
      let
        file = pkgs.writeText "nixling-${profileId}.json" (builtins.toJSON data);
      in {
        inherit data;
        path = file;
        relativePath = "minijail-profiles/${profileId}.json";
        roleProfile = toRoleProfile data;
      })
    fullProfileTable;

  # v1.1.2fu15 panel-security should-fix: detect stablePrincipalId
  # collisions at eval time. stablePrincipalId = 50000 + first-24
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
  options.nixling._bundle.minijailProfiles = lib.mkOption {
    type = lib.types.unspecified;
    readOnly = true;
    internal = true;
    description = "Internal W1 typed minijail profile artifacts keyed by profileId.";
  };

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
