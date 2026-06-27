# Host-side wiring for d2b VM audio support. Imported once at the
# top level via modules/d2b/default.nix. Materialises
#
#   - A per-VM SYSTEM service `d2b-<vm>-snd.service` that runs
#     vhost-device-sound as the per-VM d2b-<vm>-snd system user.
#     Socket at /run/d2b/vms/<vm>/snd.sock, accessible to
#     d2b-<vm>-gpu (cloud-hypervisor) via ACL on ExecStartPost.
#
#
#   - An eval-time assertion that audio.enable = true requires
#     autostart = false. autostart VMs are managed by the `microvm@`
#     system service which doesn't start d2b-<vm>-gpu.service;
#     there's no CH to connect to the audio socket.
#
#   - `systemd.tmpfiles` rules that create
#     /var/lib/d2b/vms/<vm>/state/audio-state.json for every VM with
#     `audio.enable = true`, populated with
#     `{"mic":<allowMicByDefault>,"speaker":<allowSpeakerByDefault>}`
#     on first creation. Subsequent edits via `d2b audio …` are
#     preserved (the tmpfiles 'f' type does NOT overwrite existing
#     files).
#
# Split mic/speaker enforcement is NOT done in this module in v1 —
# the design originally called for a WirePlumber stream-rule, but a
# misplaced rule in `monitor.rules` broke the host's audio output
# during implementation. The rule was removed; v1 ships with binary
# enforcement (sidecar on/off). See the let-block note below for the
# remaining work.
{ config, lib, pkgs, ... }:

let
  cfg = config.d2b;
  d2bLib = import ../../lib.nix { inherit lib; };
  enabledVms = lib.filterAttrs
    (_: vm: vm.audio.enable)
    (d2bLib.normalNixosVms cfg.vms);

  anyAudio = enabledVms != { };

  # Wayland user's UID — used to find the host compositor's
  # pipewire-0 socket and ACL-grant the per-VM audio sidecar
  # user to read/write it. Assertions guarantee waylandUser is
  # non-null whenever audio.enable is set on any VM, so the `or 0`
  # fallback only matters when this module is being evaluated for
  # an audio-less configuration (anyAudio = false, services
  # below conditioned on it).
  waylandUid =
    if cfg.site.waylandUser != null
    then toString (config.users.users.${cfg.site.waylandUser}.uid or 0)
    else "0";

  # nixpkgs ships vhost-device-sound v0.2.0 which has a known
  # PipeWire-backend format negotiation bug (audible as static on
  # any non-trivial playback). v0.3.0 (via ext/vhost-device-sound/)
  # includes the fix from upstream PR #884.
  vhostDeviceSound = import ../../../pkgs/vhost-device-sound { inherit pkgs; };

  # NOTE on WirePlumber split-direction enforcement
  #
  # An earlier iteration installed a WirePlumber config drop-in at
  # /etc/wireplumber/wireplumber.conf.d/90-d2b.conf that used
  # `monitor.rules` to match `application.name = "~d2b-*"` and
  # apply per-stream restrictions. `monitor.rules` is the WRONG
  # section — it filters discovered ALSA HARDWARE monitors, not
  # client streams. The rule put WirePlumber into a state where the
  # host's audio output devices disappeared from plasma-pa
  # entirely. It has been REMOVED.
  #
  # v1 enforces direction binary at the systemd-user-service layer
  #   both off  -> sidecar stopped, no virtio-snd device, guest has
  #                no soundcard
  #   any on    -> sidecar runs, guest sees a normal soundcard with
  #                both directions live; user can still mute either
  #                direction in plasma-pa per running d2b-<vm>
  #                stream.
  #
  # A correct stream-rule would live under `node.rules` with
  # `media.class = "Stream/Input/Audio"` and `application.process.binary
  # = "vhost-device-soun"`, OR be a Lua script under
  # ~/.config/wireplumber/scripts/. Either approach MUST be tested on
  # a scratch user before being merged here.
in

{
  config = lib.mkMerge [
    # ---------------------------------------------------------------
    # Assertion: audio VMs must be interactively launched (not
    # autostart). autostart VMs run via `microvm@<vm>.service` which
    # doesn't start d2b-<vm>-gpu.service — there's no CH to
    # connect to the audio socket. (: sidecar now runs as system
    # service d2b-<vm>-snd, not in a host user's manager.)
    # ---------------------------------------------------------------
    {
      assertions =
        lib.mapAttrsToList
          (name: vm: {
            assertion = !(vm.audio.enable && vm.autostart);
            message = ''
              d2b.vms.${name}: audio.enable = true is incompatible
              with autostart = true. The audio sidecar (d2b-${name}-snd)
              is started on demand by `d2b up ${name}`, which also
              starts d2b-${name}-gpu (CH + crosvm-gpu). With
              autostart = true the microvm@ system service would boot
              the VM without a running d2b-<vm>-gpu service — the
              vhost-device-sound socket wouldn't be ready and CH would
              fail to attach a virtio-snd device. Set autostart = false
              and launch interactively, or set audio.enable = false.
            '';
          })
          config.d2b.vms;
    }

    # ---------------------------------------------------------------
    # the per-VM
    # `d2b-<vm>-snd.service` system service template was deleted.
    # The vhost-user-sound sidecar now runs as a broker-spawned runner
    # via `SpawnRunner{role: Audio, vm_id: <vm>}` per the  Audio
    # role matrix in `docs/reference/privileges.md`. The PipeWire
    # client rules, system users, tmpfiles, and the host
    # `vhost-device-sound` package below remain because the broker
    # runner consumes them.
    # ---------------------------------------------------------------

    # ---------------------------------------------------------------
    # AUDIO-ENABLED: per-VM state-file materialisation, WirePlumber
    # rule installation. Only emitted when at least one VM has
    # audio.enable = true.
    # ---------------------------------------------------------------
    (lib.mkIf anyAudio {
      # PipeWire stream-rule that prevents auto-linking of the
      # sidecar's INPUT stream (mic direction) to any hardware
      # source.
      #
      # WHY: vhost-device-sound v0.2.0 always exposes both directions
      # (it has no `--no-input` flag). On the host, that means it
      # creates two streams per running VM
      #   - Stream/Output/Audio "d2b-<vm>"  (guest plays here ->
      #                                          mixed into host sink)
      #   - Stream/Input/Audio  "d2b-<vm>"  (sucks host mic ->
      #                                          delivered to guest)
      # The default linking policy auto-routes the second stream to
      # whatever the default audio source is. For most users that's
      # a USB headset (e.g. Plantronics), and the moment capture is
      # activated on a USB device WirePlumber switches it into
      # duplex mode. USB headsets have notoriously poor clock
      # recovery in duplex; the result is audible static on the
      # user's playback when the sidecar is alive — even when
      # nothing in the VM uses the microphone, and even when the
      # state file says `mic = "off"`.
      #
      # The fix: a PipeWire `client.conf.d/` stream-rule that sets
      # `target.object = "-1"` on the sidecar's input node at
      # creation time. WirePlumber's
      # `linking/find-defined-target.lua` hook treats the literal
      # string "-1" as "do not pick a target" and skips the stream
      # entirely. The headset stays in playback-only mode and host
      # audio is unaffected.
      #
      # IMPORTANT placement notes (we have repeatedly broken host
      # audio in this area; please read before refactoring)
      #
      # 1. The rule belongs in PIPEWIRE's `client.conf.d/`, NOT
      #    WirePlumber's `wireplumber.conf.d/`. WirePlumber's
      #    `stream.rules` section is consumed only by the
      #    `node/state-stream.lua` module for state restoration; it
      #    does NOT update live node properties at creation time.
      #    PipeWire's `client.conf` is read by each libpipewire-using
      #    process at stream-create time, and its `stream.rules`
      #    section is applied to client-created streams BEFORE the
      #    node is registered with the daemon. By the time
      #    WirePlumber sees the node it already carries
      #    `target.object = "-1"` and the linking decision is
      #    short-circuited correctly.
      #
      # 2. Match keys must be `node.name = "vhost-device-sound"` or
      #    `application.name = "~d2b-.*"`. Do NOT use
      #    `application.process.binary = "vhost-device-sound"` —
      #    that key is absent on the sidecar's streams (process
      #    metadata isn't propagated through libpipewire's client
      #    socket). The actual properties on the live node are
      #    `node.name = vhost-device-sound` and `application.name =
      #    d2b-<vm>` (which we set explicitly via PIPEWIRE_PROPS
      #    in the systemd-user service template above).
      #
      # 3. Only the INPUT direction is null-targeted. The output
      #    direction MUST remain auto-linked so guest audio reaches
      #    the host sink. Two earlier iterations tried to put
      #    matches under `monitor.rules` / `monitor.alsa.rules` —
      #    both broke host audio because those sections match
      #    HARDWARE devices, not client STREAMS. Pick the right
      #    section.
      #
      # 4. The state file's `mic` flag is currently advisory; the
      #    sidecar's mic interface is always exposed, just
      #    null-targeted. Granting mic in v1+ will require the CLI
      #    to set `target.object` on the per-VM stream via the
      #    WirePlumber metadata API (override of this rule).
      # PipeWire client-side stream rule: when libpipewire (embedded
      # in vhost-device-sound) creates the daemon's INPUT stream, this
      # rule injects `target.object = "-1"` into the stream's node
      # properties. WirePlumber's `linking/find-defined-target.lua`
      # hook recognises "-1" as the canonical sentinel for "do not
      # pick a target" and skips linking the stream entirely.
      #
      # WHY HERE (client.conf.d, not WP's stream.rules)
      # WirePlumber's `stream.rules` section is consumed ONLY by the
      # node/state-stream.lua module for state restoration; it does
      # NOT update live node properties at creation time. PipeWire's
      # `client.conf` is read by each libpipewire-using process at
      # stream-create time, and its `stream.rules` section is applied
      # to client-created streams BEFORE the node is registered with
      # the daemon. By the time WirePlumber sees the node it already
      # carries `target.object = "-1"` and the linking decision is
      # short-circuited correctly.
      #
      # WHY ONLY THE INPUT DIRECTION
      # vhost-device-sound v0.2.0 exposes both directions unconditionally
      # (no --no-input flag). The output direction must remain auto-
      # linked so guest audio reaches the host sink. Only the input
      # direction needs to be null-targeted: that's the one that puts
      # USB headsets into duplex mode and causes audible static on the
      # host's own playback. The state file's `mic` flag is currently
      # advisory; granting mic in v1+ will require the CLI to set
      # `target.object` on the per-VM stream via the WirePlumber
      # metadata API.
      #
      # WHY ALSO node.dont-reconnect
      # belt-and-suspenders. If something else (a plasma-pa user
      # action, the saved-target restore hook) tries to re-bind the
      # stream to a hardware source after our null-target takes
      # effect, dont-reconnect prevents WP's automatic reconnection
      # logic from re-establishing the link on metadata changes.
      # security-r8-audio-6: per-direction routing rules driven by the
      # custom `d2b.mic` / `d2b.speaker` PipeWire properties
      # set by the sidecar's ExecStartPre from /var/lib/d2b/vms/<vm>/
      # state/audio-state.json. application.name stays cleanly per-VM
      # ("d2b-<vm>") for human-readable wpctl/pavucontrol output.
      #
      # Capture stream blocked iff d2b.mic = "off".
      # Playback stream blocked iff d2b.speaker = "off".
      #
      # When both are off the audioArgsScript in audio.nix already
      # short-circuits and does NOT emit --generic-vhost-user, so the
      # device isn't attached to CH in the first place.
      #
      # WHY ONLY blocking when the direction is OFF
      # PipeWire's `find-defined-target.lua` short-circuits its
      # decision chain via `node.dont-fallback = true` when the
      # target is "-1". `node.linger = true` keeps the stream alive
      # in the unlinked state instead of destroying it (otherwise the
      # guest's audio device disappears mid-flight). When the
      # direction is ON we WANT the auto-route, so we MUST NOT set
      # any of these props — leave WirePlumber's normal default-
      # target hook do its job.
      services.pipewire.extraConfig.client."90-d2b" = {
        "stream.rules" = (lib.optional (cfg.site.audio.inputTargetNode != null) {
          # Capture allow: on hosts where WirePlumber does not auto-link
          # capture clients to the configured default source, force d2b
          # mic-enabled streams to the operator-declared source node.
          matches = [
            {
              "d2b.mic" = "on";
              "media.class" = "Stream/Input/Audio";
            }
          ];
          actions = {
            update-props = {
              "target.object" = cfg.site.audio.inputTargetNode;
            };
          };
        }) ++ [
          {
            # Capture block: only when the sidecar advertises d2b.mic=off.
            matches = [
              {
                "d2b.mic" = "off";
                "media.class" = "Stream/Input/Audio";
              }
            ];
            actions = {
              update-props = {
                "target.object" = "-1";
                "node.dont-reconnect" = true;
                "node.dont-fallback" = true;
                "node.linger" = true;
              };
            };
          }
          {
            # Playback block: only when the sidecar advertises d2b.speaker=off.
            matches = [
              {
                "d2b.speaker" = "off";
                "media.class" = "Stream/Output/Audio";
              }
            ];
            actions = {
              update-props = {
                "target.object" = "-1";
                "node.dont-reconnect" = true;
                "node.dont-fallback" = true;
                "node.linger" = true;
              };
            };
          }
        ];
      };

      # vhost-device-sound on the host PATH so an operator can invoke it
      # interactively for debugging.
      environment.systemPackages = [ vhostDeviceSound ];

      # State-file materialisation. systemd-tmpfiles 'f' creates the
      # file only if it doesn't exist; once the operator or the CLI has
      # written real values, this is a no-op forever. Argument is
      # the initial contents (single-line JSON).
      #
      # The state file is under a daemon-owned subdir
      # /var/lib/d2b/vms/<vm>/state/ (d2bd:d2b 0750).
      # This prevents any kvm-group process from unlinking/replacing the file.
      # ACLs for the GPU sidecar and d2b group are applied inline as 'a+'
      # tmpfiles rules so they are guaranteed present on fresh boot before
      # any runner starts (fixes the setfacl activation-script ordering race).
      #
      # The audio advisory lock is placed under /run/d2b/locks/ per the
      # ADR 0034 lockfile convention. The 'f' type creates it only if absent
      # and never unlinks it, preserving OFD lock semantics across daemon
      # restarts. d2b group traversal on /run/d2b/locks is granted inline so
      # d2b members can reach the lock without broader locks-dir access.
      systemd.tmpfiles.rules =
        let
          mk = name: vm:
            let
              mic = if vm.audio.allowMicByDefault then "on" else "off";
              spk = if vm.audio.allowSpeakerByDefault then "on" else "off";
              initial = ''{"mic":"${mic}","speaker":"${spk}"}'';
            in
            # 'd'  = create directory if missing (won't change mode of existing).
            # 'f'  = create regular file only if absent; never overwrites.
            # 'a+' = append ACL entries (idempotent; runs even on existing paths).
            #
            # state/: d2bd:d2b 0750 — daemon owns it, launcher-group traverses.
            [''d /var/lib/d2b/vms/${name}/state 0750 d2bd d2b -''
             ''f /var/lib/d2b/vms/${name}/state/audio-state.json 0640 d2bd d2b - ${initial}''
             # d2b group traversal on the VM state root so CLI users can
             # reach state/ without relying on the users group membership.
             ''a+ /var/lib/d2b/vms/${name} - - - - g:d2b:--x''
             # GPU sidecar reads audio routing state to set vhost-user-sound
             # PIPEWIRE_PROPS at spawn time.
             #
             # Access ACL on the directory allows the GPU user to traverse
             # into state/ and read the current audio-state.json inode.
             # The default ACL on state/ ensures that any replacement inode
             # written via an atomic rename (create temp file in state/,
             # rename to audio-state.json) inherits the GPU read permission
             # on the new inode; without a default ACL the access ACL on the
             # old inode is lost after the rename.
             ''a+ /var/lib/d2b/vms/${name}/state - - - - u:d2b-${name}-gpu:r-x''
             ''a+ /var/lib/d2b/vms/${name}/state/audio-state.json - - - - u:d2b-${name}-gpu:r--''
             # Default ACL: newly created files in state/ (including temp
             # files that are later renamed to audio-state.json) inherit
             # d2b-<vm>-gpu:r--. default:m::r-- caps the mask so the
             # effective permission on newly created files is exactly r--.
             ''a+ /var/lib/d2b/vms/${name}/state - - - - default:u:d2b-${name}-gpu:r--''
             ''a+ /var/lib/d2b/vms/${name}/state - - - - default:m::r--''
             # Audio advisory lock under /run/d2b/locks/ per ADR 0034.
             # Mode 0660 root:d2b so d2b members can flock it.
             # 'f' never unlinks; OFD locks survive across daemon restarts.
             ''f /run/d2b/locks/audio-${name}.lock 0660 root d2b -''];
        in
        lib.concatLists (lib.mapAttrsToList mk enabledVms)
        # Grant d2b group traversal into the locks directory so members
        # can open per-VM advisory lock files placed there.
        ++ [ ''a+ /run/d2b/locks - - - - g:d2b:--x'' ];

      # Migration-only: if the old audio-state path exists and the new path does
      # not, move it. Directory creation, posture, and ACL grants are all
      # tmpfiles-owned above (d, f, and a+ rules) so a fresh boot grants are
      # deterministic with no activation-script ordering dependency.
      system.activationScripts.d2bAudioStateDirs =
        lib.stringAfter [ "users" ] (lib.concatStringsSep "\n" (lib.mapAttrsToList
          (name: _: ''
            # One-time migration: move old audio-state.json to new path.
            old_f="/var/lib/d2b/vms/${name}/audio-state.json"
            new_f="/var/lib/d2b/vms/${name}/state/audio-state.json"
            if [ -f "$old_f" ] && [ ! -f "$new_f" ]; then
              install -D -m 0640 -o d2bd -g d2b "$old_f" "$new_f" && rm -f "$old_f" || true
            fi
          '')
          enabledVms));
    })
  ];
}
