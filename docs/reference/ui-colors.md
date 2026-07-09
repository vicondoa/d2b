# UI color contract

D2b can emit a compositor-agnostic, resolved UI color contract for
desktop components that want to share the same host, realm, workload, VM,
environment, and state colors. Enable the generic artifacts with:

```nix
d2b.site.ui.enable = true;
```

The niri backend also enables the resolved color model:

```nix
d2b.site.ui.compositors.niri.enable = true;
```

## Source options

Configure colors through typed NixOS options, not compositor-specific
strings:

```nix
d2b.site.ui.colors.hostAccent = "#89b4fa";
d2b.site.ui.colors.states.running = "#a6e3a1";

d2b.envs.work.ui.accentColor = "#ffa500";

d2b.realms.work.network.ui.accentColor = "#ffa500";

d2b.vms.workstation.ui.border = {
  activeColor = "#ffa500";
  inactiveColor = "#ffa500";
  urgentColor = "#ffa500";
};
```

Every source color must be a six-digit CSS hex string (`#rrggbb`).
Resolved artifacts normalize colors to lowercase.

## Realm colors

Each enabled realm gets an accent color entry in the resolved artifacts.
The accent color is the primary visual identity for realm-first desktop
surfaces such as realm status indicators, launcher badges, and desktop
control tools. Realm colors are **presentation metadata only** — they
carry no authorization semantics.

Set a realm accent color explicitly:

```nix
d2b.realms.work.network.ui.accentColor = "#ffa500";
```

When `network.ui.accentColor` is null, d2b derives a deterministic
palette color from the realm name. Resolved colors are always lowercase.

The realm entry in `ui-colors.json` includes the canonical realm path
(see `d2b.realms.<realm>.path`) alongside the accent so consumers can
route colors to realm-path–qualified display targets without re-reading
the realm config.

If a VM border color is omitted, d2b resolves it as follows:

| Field | Resolution |
| --- | --- |
| `active` | `ui.border.activeColor`, then the deprecated niri color compatibility input, then a deterministic VM-name palette color |
| `inactive` | `ui.border.inactiveColor`, then the resolved active color |
| `urgent` | `ui.border.urgentColor`, then the resolved active color |

The inactive default intentionally preserves VM identity coloring when a
window loses focus. Operators who prefer a neutral inactive border should
set `d2b.vms.<vm>.ui.border.inactiveColor` once in d2b instead of
adding compositor-local override rules.

## Wayland proxy borders

For graphics VMs using the host-side Wayland proxy, and for qemu-media host
windows routed through that proxy, d2b can draw the VM identity border inside
`d2b-wayland-proxy` itself. Proxy-drawn borders are enabled by default when the
proxy is active and can be disabled per VM:

```nix
d2b.vms.work.graphics.waylandProxy.border.enable = false;
```

The proxy uses the resolved realm color when a VM maps unambiguously to a
realm workload, falling back to the resolved VM border colors documented
above. It exposes a proxy-owned wrapper toplevel and draws only proxy-owned
rail pixels; guest application buffers and dma-bufs stay forwarded as Wayland
objects and file descriptors and are not copied or sampled to render the rail.
The default identity rail appears on the left side with the
`<workload>.<realmPath>` label when realm workload identity is available, and
with the authenticated VM-name label otherwise.

Proxy borders are visual-only. They do not intercept pointer input, and
popup/menu positioning remains based on the guest surface geometry.

The default label is the authenticated VM name. Disable the label while keeping
the colored border with:

```nix
d2b.vms.work.graphics.waylandProxy.border.label.enable = false;
```

## JSON artifact

When enabled, d2b writes `/etc/d2b/ui-colors.json` by default.
The JSON schema is committed at
[`ui-colors-schema.json`](./ui-colors-schema.json).

Shape:

```json
{
  "version": 1,
  "host": { "accent": "#89b4fa" },
  "states": {
    "running": "#a6e3a1",
    "transitioning": "#f9e2af",
    "pendingRestart": "#fab387",
    "error": "#f38ba8",
    "denied": "#cba6f7",
    "unknown": "#6c7086"
  },
  "envs": {
    "work": { "accent": "#ffa500" }
  },
  "vms": {
    "workstation": {
      "env": "work",
      "border": {
        "active": "#ffa500",
        "inactive": "#ffa500",
        "urgent": "#ffa500"
      }
    }
  },
  "realms": {
    "work": {
      "path": "work",
      "accent": "#ffa500"
    },
    "payments": {
      "path": "payments.work",
      "accent": "#7fc8ff"
    }
  }
}
```

The `realms` object is keyed by realm id. Each entry includes:

- `path` — the canonical realm path (`d2b.realms.<realm>.path`), written
  most-specific-first (e.g. `payments.work` for a realm whose `parent` is
  `work`). Desktop consumers use this to route the accent to
  realm-path–qualified display targets.
- `accent` — the resolved accent color. Set via
  `d2b.realms.<realm>.network.ui.accentColor`; falls back to a
  deterministic palette color derived from the realm name.

Realm colors are presentation metadata only and carry no authorization
semantics.

Consumers should fail visibly but remain usable if the default artifact is
missing or malformed. Do not treat the artifact as an authorization source;
it is presentation metadata only.

## GTK CSS artifact

D2b also writes `/etc/d2b/ui-colors.css` by default. It exposes
GTK-compatible `@define-color` declarations for bars, launchers, and
compositor-adjacent stylesheets:

```css
@import url("/etc/d2b/ui-colors.css");

#realm-work {
  border-left-color: @d2b_realm_work_accent;
}
```

Color definitions use underscores so GTK/Waybar parsers accept them.
Definition names include:

| Definition | Meaning |
| --- | --- |
| `d2b_host_accent` | Host identity accent |
| `d2b_state_running` | Running state accent |
| `d2b_state_transitioning` | Starting/stopping state accent |
| `d2b_state_pending_restart` | Pending-restart state accent |
| `d2b_state_error` | Error state accent |
| `d2b_state_denied` | Authorization-denied state accent |
| `d2b_state_unknown` | Unknown/unavailable state accent |
| `d2b_realm_<realm>_accent` | Realm identity accent |
| `d2b_env_<env>_accent` | Environment identity accent for the transition substrate |
| `d2b_vm_<vm>_border_active` | VM active border color |
| `d2b_vm_<vm>_border_inactive` | VM inactive border color |
| `d2b_vm_<vm>_border_urgent` | VM urgent border color |

Hyphens in environment, realm, and VM names are emitted as underscores in the CSS
definition name, for example VM `work-aad` becomes
`d2b_vm_work_aad_border_active` and realm `my-work` becomes
`d2b_realm_my_work_accent`.

## Niri backend

Enable niri output with:

```nix
d2b.site.ui.compositors.niri.enable = true;
```

D2b writes `/etc/d2b/niri-vm-borders.kdl` by default. The KDL
renders `active-color`, `inactive-color`, and `urgent-color` from the same
resolved VM border model used by the JSON and GTK CSS artifacts.

The niri backend remains useful for host windows that do not pass through
`d2b-wayland-proxy`, and for operators who deliberately want an additional
compositor-native wrapper. Wayland toplevels routed through the host-side proxy
get compositor-agnostic proxy borders by default; the proxy is the primary owner
of the VM identity border for those windows.

Do not enable or include the niri artifact solely to color proxied graphics or
qemu-media windows. If you include it anyway, niri-native borders and focus
rings wrap the proxy rail and guest content together as an outer compositor
decoration.

The legacy `d2b.site.niriVmBorders` and `d2b.vms.<vm>.graphics.niriBorderColor`
options remain compatibility inputs for one release, but new
configuration should use the `d2b.site.ui.*`,
`d2b.envs.<env>.ui.*`, and `d2b.vms.<vm>.ui.*` paths.
