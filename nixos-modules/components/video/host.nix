# Host-side wiring for nixling VM hardware video decode.
#
# Creates a per-VM SYSTEM service `nixling-<vm>-video.service` that runs
# `crosvm device video-decoder` with the VA-API backend (nvidia-vaapi-driver
# → NVDEC). Socket at /run/nixling/vms/<vm>/video.sock, accessible to
# the GPU sidecar (cloud-hypervisor).
{ config, lib, pkgs, ... }:

let
  cfg = config.nixling;
  # Only graphics VMs get video decode (needs virtio-gpu for presentation).
  enabledVms = lib.filterAttrs
    (_: vm: vm.enable && vm.graphics.enable)
    cfg.vms;

  anyVideo = enabledVms != { };

  # The crosvm binary with video-decoder + vaapi features.
  # Note: cargoBuildFeatures is the internal attr name used by
  # rustPlatform.buildRustPackage for cargo --features.
  crosvmVideo = (pkgs.crosvm.overrideAttrs (old: {
    buildInputs = (old.buildInputs or []) ++ [ pkgs.libva ];
    cargoBuildFeatures = (old.cargoBuildFeatures or old.buildFeatures or []) ++ [
      "video-decoder" "vaapi" "media"
    ];
    cargoCheckFeatures = (old.cargoCheckFeatures or old.cargoBuildFeatures or old.buildFeatures or []) ++ [
      "video-decoder" "vaapi" "media"
    ];
    postPatch = (old.postPatch or "") + ''
      # Create vhost-user video decoder backend (inline, avoids patch compat issues)
      mkdir -p devices/src/virtio/vhost_user_backend/video/sys
      cp ${../../../pkgs/vhost-user-video/mod.rs} devices/src/virtio/vhost_user_backend/video/mod.rs
      cp ${../../../pkgs/vhost-user-video/sys_mod.rs} devices/src/virtio/vhost_user_backend/video/sys/mod.rs
      cp ${../../../pkgs/vhost-user-video/sys_linux.rs} devices/src/virtio/vhost_user_backend/video/sys/linux.rs

      # Register video module in vhost_user_backend
      substituteInPlace devices/src/virtio/vhost_user_backend/mod.rs \
        --replace-fail \
          '#[cfg(feature = "audio")]
pub mod snd;' \
          '#[cfg(feature = "audio")]
pub mod snd;
#[cfg(feature = "video-decoder")]
pub mod video;'

      substituteInPlace devices/src/virtio/vhost_user_backend/mod.rs \
        --replace-fail \
          '#[cfg(feature = "audio")]
pub use snd::run_snd_device;
#[cfg(feature = "audio")]
pub use snd::Options as SndOptions;' \
          '#[cfg(feature = "audio")]
pub use snd::run_snd_device;
#[cfg(feature = "audio")]
pub use snd::Options as SndOptions;
#[cfg(feature = "video-decoder")]
pub use video::sys::run_video_device;
#[cfg(feature = "video-decoder")]
pub use video::sys::Options as VideoOptions;'

      # Add VideoDecoder to CrossPlatformDevicesCommands
      substituteInPlace src/crosvm/cmdline.rs \
        --replace-fail \
          '#[cfg(feature = "audio")]
    Snd(vhost_user_backend::SndOptions),
}' \
          '#[cfg(feature = "audio")]
    Snd(vhost_user_backend::SndOptions),
    #[cfg(feature = "video-decoder")]
    VideoDecoder(vhost_user_backend::VideoOptions),
}'

      # Add import and dispatch in main.rs
      substituteInPlace src/main.rs \
        --replace-fail \
          '#[cfg(feature = "audio")]
use devices::virtio::vhost_user_backend::run_snd_device;' \
          '#[cfg(feature = "audio")]
use devices::virtio::vhost_user_backend::run_snd_device;
#[cfg(feature = "video-decoder")]
use devices::virtio::vhost_user_backend::run_video_device;'

      substituteInPlace src/main.rs \
        --replace-fail \
          '#[cfg(feature = "audio")]
            CrossPlatformDevicesCommands::Snd(cfg) => run_snd_device(cfg),
        },' \
          '#[cfg(feature = "audio")]
            CrossPlatformDevicesCommands::Snd(cfg) => run_snd_device(cfg),
            #[cfg(feature = "video-decoder")]
            CrossPlatformDevicesCommands::VideoDecoder(cfg) => run_video_device(cfg),
        },'

      # Make resource_bridge optional in Decoder (clean patch from actual source)
      patch -p1 < ${../../../pkgs/patches/crosvm-decoder-optional-bridge.patch}

      # Make media.rs types public for the vhost-user video backend
      substituteInPlace devices/src/virtio/media.rs \
        --replace-fail 'struct EventQueue(Queue);' 'pub struct EventQueue(pub Queue);' \
        --replace-fail 'struct WaitContextPoller(Rc<WaitContext<Token>>);' 'pub struct WaitContextPoller(pub Rc<WaitContext<Token>>);' \
        --replace-fail 'enum Token {' 'pub enum Token {' \
        --replace-fail 'struct HostMemoryMapper<M: SharedMemoryMapper> {' 'pub struct HostMemoryMapper<M: SharedMemoryMapper> {' \
        --replace-fail '    shm_mapper: M,' '    pub shm_mapper: M,' \
        --replace-fail '    allocator: AddressAllocator,' '    pub allocator: AddressAllocator,'

      # Fix todo!() panics in decoder_adapter — log and skip instead of crash
      substituteInPlace devices/src/virtio/media/decoder_adapter.rs \
        --replace-fail 'DecoderEvent::ResetCompleted(_) => todo!(),' \
                        'DecoderEvent::ResetCompleted(_) => { base::warn!("ResetCompleted unhandled"); None }' \
        --replace-fail 'DecoderEvent::NotifyError(_) => todo!(),' \
                        'DecoderEvent::NotifyError(e) => { base::error!("decoder error: {e:?}"); None }'
    '';
  }));
in
{
  config = lib.mkIf anyVideo {

    systemd.services = lib.mapAttrs' (name: vmCfg: lib.nameValuePair "nixling-${name}-video" {
      description = "vhost-user video decoder sidecar for nixling VM ${name}";
      wantedBy = [ ];
      partOf = [ "nixling-${name}-gpu.service" ];
      restartIfChanged = false;

      serviceConfig = {
        # Run as the GPU sidecar user (needs /dev/dri + NVIDIA access).
        User = "nixling-${name}-gpu";
        Group = "nixling-${name}-gpu";

        RuntimeDirectory = "nixling-video/${name}";
        RuntimeDirectoryMode = "0750";

        ExecStart = lib.concatStringsSep " " [
          "${crosvmVideo}/bin/crosvm"
          "device"
          "video-decoder"
          "--socket-path" "/run/nixling-video/${name}/video.sock"
          "--backend" "vaapi"
        ];

        # Type=notify would be ideal but crosvm doesn't sd_notify.
        # Use forking-like approach: the GPU sidecar (CH) checks for
        # the socket in its extraArgs script before connecting.
        Type = "simple";
        
        Environment = [
          "LIBVA_DRIVER_NAME=nvidia"
          "LIBVA_DRIVERS_PATH=/run/opengl-driver/lib/dri"
          "NV_VAAPI_BACKEND=direct"
          "LD_LIBRARY_PATH=/run/opengl-driver/lib"
        ];

        DeviceAllow = [
          "/dev/dri/renderD128 rw"
          "char-195:* rw"
          "char-510:* rw"
          "/dev/nvidiactl rw"
          "/dev/nvidia0 rw"
          "/dev/nvidia-uvm rw"
        ];

        DevicePolicy = "closed";
        ProtectSystem = "strict";
        ProtectHome = true;
        NoNewPrivileges = true;

        Restart = "on-failure";
        RestartSec = "2s";
      };
    }) enabledVms;
  };
}
