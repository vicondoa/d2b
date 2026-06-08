{ config, lib, pkgs, ... }:

let
  cfg = config.nixling;
  enabledVms = lib.filterAttrs (_: vm: vm.enable) cfg.vms;
  obsOtlpPort = 14317;
  serviceControllers = [ "cpu" "memory" "pids" ];

  privateEtc = source: {
    inherit source;
    mode = "0640";
    user = "root";
    group = if cfg.daemonExperimental.enable then "nixlingd" else "root";
  };

  stablePrincipalId = principal:
    if principal == "root" then 0
    else 50000 + lib.fromHexString (builtins.substring 0 6 (builtins.hashString "sha256" principal));

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
    }:
    {
      inherit
        profileId
        role
        capabilities
        namespaces
        seccompPolicyRef
        requiresStartRoot
        exceptionRef
        adr_carve_out
        ;
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
  };

  profileIdFor = name: nodeId: "vm-${name}-${nodeId}";
  stateDirOf = name: "${toString cfg.store.stateDir}/${name}";
  runtimeDirOf = name: "/run/nixling/${name}";
  audioRuntimeDirOf = name: "/run/nixling/vms/${name}";
  videoRuntimeDirOf = name: "/run/nixling-video/${name}";
  gpuRuntimeDirOf = name: "/run/nixling-gpu/${name}";
  swtpmRuntimeDirOf = name: "/run/swtpm/${name}";
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
      virtiofsdRootException = "ADR 0003 virtiofsd --sandbox=namespace setup exception";
      virtiofsShares = lib.filter
        (share: (share.proto or "virtiofs") == "virtiofs")
        config.microvm.vms.${name}.config.config.microvm.shares;
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
            capabilities = [
              "CAP_SYS_ADMIN"
              "CAP_SETPCAP"
              "CAP_CHOWN"
              "CAP_FOWNER"
              "CAP_FSETID"
              "CAP_SETUID"
              "CAP_SETGID"
              "CAP_DAC_OVERRIDE"
              "CAP_MKNOD"
              "CAP_SETFCAP"
            ];
            seccompPolicyRef = "w1-virtiofsd";
            readOnlyPaths = [ "/nix/store" ];
            writablePaths = [
              (mkWritablePath (stateDirOf name) "Materialize virtiofs sockets and VM-local store state.")
              (mkWritablePath (runtimeDirOf name) "Expose broker-prepared virtiofs runtime sockets.")
            ];
            cgroupSubtree = "nixling.slice/${name}/${shareNodeId}";
            controllers = serviceControllers;
            requiresStartRoot = true;
            exceptionRef = virtiofsdRootException;
            adr_carve_out = virtiofsdRootException;
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
        # P1 ph1-p1-device-matrix: Audio binds nothing under /dev; the
        # only host-side resource it talks to is the PipeWire client
        # socket at /run/user/<uid>/pipewire-0 (RO traverse — the host
        # ACL grants the per-VM principal rw on the socket inode itself
        # per components/audio/host.nix).
        readOnlyPaths = [ "/run/user" ];
        writablePaths = [
          (mkWritablePath "${stateDirOf name}/state" "Read and persist the VM audio grant state.")
          (mkWritablePath (audioRuntimeDirOf name) "Create the PipeWire-backed vhost-user audio socket at /run/nixling/vms/<vm>/snd.sock.")
        ];
        cgroupSubtree = "nixling.slice/${name}/audio";
        controllers = serviceControllers;
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
  };
}
