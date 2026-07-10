# ADR 0044: Unsafe-local runtime provider

- Status: Accepted
- Date: 2026-07-09
- Related: [ADR 0025](0025-wayland-proxy-host-jailed-role.md)
  (host-jailed Wayland filter proxy role), [ADR 0037](0037-local-hypervisor-runtime-seam.md)
  (local hypervisor runtime seam), [ADR 0038](0038-persistent-guest-shell-sessions.md)
  (persistent named guest shell sessions), [ADR 0039](0039-constellation-persistent-shell-routing.md)
  (constellation persistent shell routing), [ADR 0042](0042-d2b-clipboard-authority-and-picker-split.md)
  (clipboard authority and picker split), [ADR 0043](0043-realm-native-control-plane.md)
  (realm-native control plane)

## Context

The realm-native desktop model makes VM and provider workloads first-class:
Waybar, wlcontrol, wlterm, clip-picker, and Wayland window rails all consume
realm/workload identity metadata instead of bespoke host-local groups. The
remaining awkward surface is the physical host itself. Host launchers such as
the app drawer, browser, and terminal are currently styled as a special Waybar
host group, while VM/provider workloads use d2b workload metadata.

That split is visible in both implementation and user experience:

- host apps can be grouped and styled, but they are not targetable d2b
  workloads;
- host-launched Wayland windows do not automatically carry d2b identity rails;
- wlcontrol and wlterm cannot present local host applications and local host
  shell sessions through the same workload/provider model as realm VMs;
- adding more host shortcuts risks rebuilding a second "host launcher" model
  beside the realm workload model selected by [ADR 0043](0043-realm-native-control-plane.md).

At the same time, the host is not a safe execution environment in the same
sense as a microVM, gateway VM, provider sandbox, or remote d2b node. A process
started directly in the operator's host user session can read the user's host
files, share the host network namespace, talk to host D-Bus and portals, see
ambient credentials, and interact with unrelated host processes according to
normal OS permissions. It is useful, but it is not isolated.

The design therefore needs a way to make local host launchers first-class
desktop workloads without implying security properties that do not exist.

## Decision

D2b will add an explicit runtime provider kind named **`unsafe-local`**. An
unsafe-local workload runs directly as the configured host user in the host user
session and inherits the host user's normal namespaces. It is a first-class
realm workload for desktop presentation, launch, target resolution, and shell
UX, but it is **not** an isolation boundary.

The name is intentionally blunt. Any UI, CLI, machine-readable status, docs, or
schema that exposes the provider kind must preserve the `unsafe-local` label and
must not rename it to a friendlier term that hides the security posture.

Unsafe-local workloads are useful for:

- host browser/editor/terminal launchers that should appear beside VM/provider
  workloads in a realm card;
- host-side "local tools" realms, such as a red host/local realm rail for
  unconfined windows;
- persistent host shell sessions presented through wlterm with the same
  create/list/open/stop UX as guest shell sessions;
- migration away from bespoke host boxes and toward one workload metadata model
  consumed by Waybar, wlcontrol, wlterm, clip-picker, and Wayland rails.

Unsafe-local workloads are not useful for:

- security isolation;
- work/personal credential separation;
- filesystem containment;
- network segmentation;
- policy enforcement against a malicious host process.

## Security and threat model

Unsafe-local is a presentation and convenience provider only. It makes host
processes more visible and consistently labeled; it does not make them safer.

An unsafe-local process:

- runs as the configured host user, never as root through d2b;
- inherits the host user session's normal mount, PID, IPC, UTS, cgroup, and
  network namespace context without d2b unsharing or claiming namespace
  isolation;
- sees the host filesystem according to the user's normal OS permissions;
- may access the user's D-Bus session, portals, SSH/GPG agents, browser
  profiles, keyrings, and other ambient user-session resources unless the
  launched program itself avoids them;
- may interact with other host user processes according to normal OS
  permissions;
- does not have a guestd, vsock boundary, virtual NIC, d2b net VM, TPM boundary,
  swtpm state, virtiofs store-view, USBIP mediation, or VM lifecycle DAG.

Consequently:

- `unsafe-local` must never satisfy a policy requirement for isolated,
  gateway-backed, provider-managed, work-managed, or credential-separated
  execution.
- `unsafe-local` must not be shown as equivalent to `local-vm`,
  `qemu-media`, `aca-sandbox`, `cloud-full-host`, or any future isolated
  runtime.
- Any realm policy that permits unsafe-local workloads must do so explicitly.
  There is no implicit fallback from an unavailable isolated runtime to
  unsafe-local.
- A relay-authenticated or remote identity is never mapped to local host-user
  unsafe-local execution.
- Audit and telemetry must label the provider as `unsafe-local` while avoiding
  command argv, environment, cwd, shell transcript, or host path leakage.

The red host/local rail is a warning and identity cue, not an access-control
mechanism.

## Workload declaration shape

Unsafe-local is declared as a normal realm workload provider kind under the
realm-native workload surface:

```nix
d2b.realms.host.workloads.browser = {
  kind = "unsafe-local";
  launcher = {
    label = "Host browser";
    icon.name = "language";
  };
  command = [ "firefox" ];
};

d2b.realms.host.workloads.terminal = {
  kind = "unsafe-local";
  launcher = {
    label = "Host terminal";
    icon.name = "terminal";
  };
  shell = {
    enable = true;
    defaultName = "host";
  };
};
```

The concrete option names may evolve during implementation, but the contract is:

- the workload has a stable workload id and canonical target
  `<workload>.<realm>.d2b`;
- it carries the same `launcher` desktop metadata shape as other workloads:
  label, icon id/name, provider kind, and capabilities;
- realm rail color comes from realm UI metadata such as
  `d2b.realms.<realm>.network.ui.accentColor`, not from a per-workload color
  field;
- command declarations are data, not free-form shell strings;
- shell-capable unsafe-local workloads advertise shell capability through the
  same status/list surfaces used by wlterm for other workloads;
- machine-readable status includes a typed isolation posture field indicating
  that the workload has no isolation boundary. The field must be a closed enum
  or structured object (for example `isolationPosture = "unsafeLocal"` plus a
  human-facing warning string), not an ad hoc free-form warning that downstream
  tools must parse.

Host-local shortcuts that are not declared as unsafe-local workloads may remain
ordinary user configuration, but d2b desktop tools should prefer declared
unsafe-local workload metadata over bespoke host launcher lists.

## Launch model

Unsafe-local launch is a user-session operation. It must not require privileged
broker mutation and must not spawn host applications as root. Because `d2bd`
runs as the framework daemon user, it must also not directly impersonate an
arbitrary host user or guess a user D-Bus address. The implementation requires a
defined user-session IPC seam.

The launcher path is:

```text
wlcontrol / Waybar / CLI
  -> d2bd realm workload launch operation
    -> unsafe-local provider adapter
      -> per-user unsafe-local session helper
        -> user-session scope
          -> d2b-wayland-proxy socket
            -> host Wayland compositor
          -> launched command with WAYLAND_DISPLAY=<proxy-socket>
```

Requirements:

- A per-user unsafe-local helper runs inside the target user's graphical user
  manager and authenticates to `d2bd` with the same local operator identity
  model used by other desktop helpers.
- Unsafe-local requires an active graphical `systemd --user` session with the
  usual PAM and D-Bus user-manager integration. If the user manager or D-Bus
  session is unavailable, provider status must report the missing prerequisite
  and launch must fail visibly.
- `d2bd` records typed launch intents and the helper pulls or receives only
  intents authorized for that local uid/session. The helper owns access to the
  user D-Bus, user systemd manager, inherited desktop environment, and host
  compositor socket.
- `d2bd` may refuse launch when no matching user-session helper is registered
  for the operator/session. It must not fall back to root, `sudo`, `su`,
  guessed D-Bus addresses, or broker-mediated arbitrary command execution.
- The process runs in a transient user systemd scope owned by the helper.
  POSIX process groups alone are forbidden as the teardown authority because
  they cannot reliably account for descendants that escape or reparent.
- The process inherits only the environment explicitly selected by the
  implementation. A minimal allowlist is preferred; if the first implementation
  uses the user manager's ambient environment, that limitation must be
  documented and visible in provider status.
- `WAYLAND_DISPLAY` for GUI launch points at a d2b-owned Wayland proxy socket,
  not directly at the host compositor.
- The proxy applies the workload's realm color rail, app-id prefix, title
  prefix, and clipboard bridge policy where available.
- The provider must fail visibly if the Wayland proxy cannot be started or if
  the target command cannot connect through it. It must not silently relaunch
  directly on the host compositor.
- Non-Wayland commands may be supported only as explicitly non-graphical
  unsafe-local commands. They do not receive a rail and must not be presented as
  rail-protected windows.

The initial rail color for the host/local realm should be red by convention, but
the color remains metadata. It is not an authorization primitive.

## Shell model

Unsafe-local shell-capable workloads must integrate with the same operator
verbs and desktop UX as existing persistent workload shells:

```text
d2b shell host.host.d2b
d2b shell host.host.d2b list
d2b shell host.host.d2b open <name>
d2b shell host.host.d2b stop <name>
```

and wlterm must be able to:

- discover unsafe-local shell-capable workloads;
- create a named host shell session;
- list existing host shell sessions;
- open an existing host shell session;
- stop a host shell session;
- display unsafe-local shell state in realm groups with the provider warning.

The provider must implement shell operations as semantic d2b shell operations,
not as ad hoc terminal shortcuts. The implementation may reuse the existing
persistent shell session model from [ADR 0038](0038-persistent-guest-shell-sessions.md)
and [ADR 0039](0039-constellation-persistent-shell-routing.md), but the
execution backend is host-local:

```text
unsafe-local shell manager
  -> per-user unsafe-local session helper
  -> host user shell/session supervisor
  -> PTY/session record owned by the host user helper
  -> terminal open through d2b-wayland-proxy
```

Shell requirements:

- Shell commands run as the configured host user, never as root.
- Shell sessions are named and bounded by the same session-table and quota
  concepts used for other shell-capable workloads, adapted to host-local state.
- Opening a shell in a GUI terminal must route the terminal window through
  `d2b-wayland-proxy` so it receives the realm rail and title/app-id prefix.
- Shell transcripts, argv, cwd, and environment are never logged, audited, or
  exposed as metric labels. User-provided shell session names are also forbidden
  as metric labels; use bounded provider/workload ids and event kinds instead.
- Shell lifecycle events must be observable as bounded event boundaries:
  create, open, disconnect, stop requested, graceful exit, forced kill, and
  failure. Events carry provider/workload/session ids and result classes, not
  transcripts, argv, env, cwd, or user-provided shell names.
- Stop/kill semantics must be explicit and idempotent. A host shell stop may
  terminate only the provider-owned session scope/cgroup, not arbitrary host
  user processes. The implementation must use transient user systemd scopes for
  each provider-owned shell/session and use scope/cgroup identity for teardown.
  The teardown sequence is: close the provider-owned PTY master so normal shells
  see SIGHUP and can exit, wait the configured grace interval, send SIGTERM to
  the session scope, then SIGKILL the same scope if it remains alive. No step may
  target an arbitrary PID outside the provider-owned scope.
- Existing terminal-v1/wlterm UX should not need a separate "host terminal"
  code path. It should consume advertised shell capabilities and provider kind.

This is not a replacement for guest-control exec in VMs. It is a host-local
provider implementation of the same shell UX for cases where the operator
intentionally wants a host shell with a d2b identity rail.

## Desktop tools

Desktop companions consume unsafe-local workloads through the same public
metadata used for other workloads.

Waybar:

- host launchers become unsafe-local workload launchers inside a host/local
  realm group;
- terminal/browser/app icons come from workload metadata or configurable
  host-local defaults, not hardcoded module definitions;
- the host group rail uses the unsafe-local realm color.

wlcontrol:

- unsafe-local workloads appear in realm cards with an unmistakable
  `unsafe-local` provider label or warning tooltip;
- warning presentation should be ambient and non-intrusive: a red realm rail,
  static provider label, and tooltip/status text are preferred over repetitive
  modal or launch-blocking dialogs that would train users to ignore the warning;
- if every workload in a card is unsafe-local, the card-level provider label is
  sufficient; per-row labels are required only when a card mixes unsafe-local
  and isolated/provider workloads;
- launch buttons call the standard workload launch operation;
- the compact row can show terminal/browser/quick-launch actions just like
  other workloads;
- lifecycle controls that do not apply to host processes are absent or disabled
  with clear capability denials.

wlterm:

- unsafe-local shell-capable workloads appear in the same realm grouping as
  other shell-capable workloads;
- create/open/stop operations route through the unsafe-local shell provider;
- opened terminal windows use the Wayland proxy rail.

clip-picker and Wayland proxy:

- unsafe-local windows are a local realm endpoint for clipboard policy and
  presentation. Clipboard authority still belongs to d2b-clipd per
  [ADR 0042](0042-d2b-clipboard-authority-and-picker-split.md).
- Direct host/VM clipboard offers remain discovery-only unless selected through
  the picker and fulfilled by d2b-clipd.
- The Wayland proxy label must make host-local unsafe workload identity visible
  without implying containment.

## Control-plane boundaries

Unsafe-local should start as a user-session provider adapter behind the existing
realm workload operation surface. It should not add a new root service or
privileged broker operation merely to start host user applications.

The provider may need daemon-visible state so CLI, Waybar, wlcontrol, and
wlterm agree on workload status and shell sessions. That state must be scoped to
unsafe-local provider records and must not become a second host process manager
for arbitrary user commands.

Acceptable first implementation:

- d2bd owns provider metadata, capability reporting, and operation routing;
- a per-user unsafe-local helper, running under the target user manager, owns
  host process start, user systemd scopes, PTY/session records, Wayland proxy
  sockets, and shell session state;
- d2b-wayland-proxy remains the only Wayland path for GUI launches;
- no privileged broker mutation is used except for already-existing generic
  read-only metadata paths, if needed.

Forbidden first implementation:

- a root daemon that launches arbitrary host user commands;
- a broker op accepting free-form command strings;
- a fallback that bypasses the Wayland proxy when proxy setup fails;
- presenting unsafe-local as isolated or work-safe;
- storing provider credentials or realm secrets on the host because
  unsafe-local exists.
- a cross-uid shortcut where `d2bd` directly spawns or supervises host user
  processes without the authenticated user-session helper.

## Migration and compatibility

The current host Waybar group can be migrated into unsafe-local declarations:

| Current shortcut | Unsafe-local workload |
| --- | --- |
| host app drawer | `apps.host.d2b` |
| host terminal | `terminal.host.d2b` |
| host browser | `browser.host.d2b` |
| host-local shell session | `shell.host.d2b` or a shell-capable `terminal.host.d2b` workload |

The migration should be additive at first:

1. Add unsafe-local workload declarations and metadata.
2. Teach desktop tools to prefer unsafe-local workload metadata when present.
3. Keep host-specific Waybar configuration as a compatibility fallback.
4. Remove bespoke host launcher wiring once unsafe-local is stable and covered
   by tests.
5. Update consumer-facing how-to and reference documentation to explain the
   migration from legacy host Waybar groups to unsafe-local workloads, including
   the no-isolation warning and the shell UX.

Host-side Home Manager integration must be explicit. The existing guest
Home Manager component does not generate host Waybar modules. The implementation
must add a host-side integration path that reads the evaluated unsafe-local
workload metadata (or the generated public launcher artifact) and renders host
Waybar modules from the same `launcher` metadata used by sibling desktop
tools. This integration must preserve user overrides for icons, labels, and
commands.

The current realm accent option is under `d2b.realms.<realm>.network.ui`.
Unsafe-local should consume that existing field for consistency with the
current UI color contract, even though host-local workloads do not create d2b
network substrate. A future schema cleanup may move presentation colors to a
realm-level UI namespace; unsafe-local must follow the canonical UI color
contract when that happens.

This keeps existing host launchers working while eliminating the long-term host
box special case.

## Consequences

Positive:

- Host launchers become first-class realm workloads instead of bespoke Waybar
  modules.
- Host windows get the same d2b identity rail and title/app-id labeling as
  VM/provider windows.
- wlterm can manage host shell sessions through the same shell UX used for
  other shell-capable workloads.
- The unsafe security posture is explicit in names, status, docs, and policy.
- Desktop tools can converge on one metadata model for host, VM, and provider
  workloads.

Negative:

- The provider is intentionally unsafe; users may still misread presentation
  consistency as isolation unless UI warnings are clear.
- A user-session process manager and shell-session store add implementation
  surface outside guestd.
- Host user environment inheritance is hard to make both convenient and
  minimal.
- Wayland-only rail coverage leaves non-Wayland commands without a visual rail.

## Implementation notes and validation

Implementation should be staged:

1. Define unsafe-local workload schema and capability metadata.
2. Add provider status and launch operations with no proxy bypass.
3. Route GUI launch through d2b-wayland-proxy and add app-id/title/rail tests.
4. Add shell session list/create/open/stop operations for unsafe-local.
5. Update wlcontrol, wlterm, Waybar integration, and clip-picker metadata
   handling.
6. Retire bespoke host launcher wiring after compatibility soak.

Validation must include:

- Nix eval tests proving unsafe-local is explicit and never an implicit
  fallback;
- schema/contract tests for provider kind, warning metadata, launch metadata,
  and shell capability reporting;
- contract tests proving unsafe-local uses the existing `launcher` metadata
  shape and realm-level UI accent color rather than a divergent display/color
  schema;
- DTO tests for the typed isolation posture field consumed by CLI, wlcontrol,
  wlterm, Waybar helpers, and any JSON status clients;
- user-session helper authorization tests proving `d2bd` cannot launch
  unsafe-local workloads without a registered helper for the requesting local
  uid/session;
- host NixOS/Home Manager tests proving unsafe-local workload metadata can
  generate host Waybar modules without using guest-only Home Manager paths;
- user-session prerequisite tests proving provider status and launch errors are
  clear when `systemd --user` or D-Bus user-manager integration is unavailable;
- Wayland proxy argv/behavior tests proving no direct-host-compositor fallback;
- wlcontrol and wlterm tests for unsafe-local workload display and shell
  operations;
- policy tests proving isolated/work-managed realms do not accept unsafe-local
  unless explicitly declared;
- shell teardown tests proving stop/kill closes the provider-owned PTY master,
  then targets only the provider-owned user systemd scope/cgroup and cannot kill
  unrelated host user processes;
- observability tests proving user-provided shell names, argv, env, cwd, paths,
  and transcripts never become metric labels or audit payloads;
- screenshot review for host realm rails once desktop integration lands.

## Non-goals

- Providing a sandbox for host apps.
- Replacing local-vm or gateway-vm for sensitive work.
- Running arbitrary host commands as root.
- Adding SSH, raw shell tunnels, or provider-specific shell channels.
- Making the Wayland rail an authorization mechanism.
