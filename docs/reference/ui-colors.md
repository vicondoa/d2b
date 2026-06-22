# UI color contract

Nixling can emit a compositor-agnostic, resolved UI color contract for
desktop components that want to share the same host, environment, VM, and
state colors. Enable the generic artifacts with:

```nix
nixling.site.ui.enable = true;
```

The niri backend also enables the resolved color model:

```nix
nixling.site.ui.compositors.niri.enable = true;
```

## Source options

Configure colors through typed NixOS options, not compositor-specific
strings:

```nix
nixling.site.ui.colors.hostAccent = "#89b4fa";
nixling.site.ui.colors.states.running = "#a6e3a1";

nixling.envs.work.ui.accentColor = "#ffa500";

nixling.vms.workstation.ui.border = {
  activeColor = "#ffa500";
  inactiveColor = "#ffa500";
  urgentColor = "#ffa500";
};
```

Every source color must be a six-digit CSS hex string (`#rrggbb`).
Resolved artifacts normalize colors to lowercase.

If a VM border color is omitted, nixling resolves it as follows:

| Field | Resolution |
| --- | --- |
| `active` | `ui.border.activeColor`, then the deprecated niri color compatibility input, then a deterministic VM-name palette color |
| `inactive` | `ui.border.inactiveColor`, then the resolved active color |
| `urgent` | `ui.border.urgentColor`, then the resolved active color |

The inactive default intentionally preserves VM identity coloring when a
window loses focus. Operators who prefer a neutral inactive border should
set `nixling.vms.<vm>.ui.border.inactiveColor` once in nixling instead of
adding compositor-local override rules.

## JSON artifact

When enabled, nixling writes `/etc/nixling/ui-colors.json` by default.
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
  }
}
```

Consumers should fail visibly but remain usable if the default artifact is
missing or malformed. Do not treat the artifact as an authorization source;
it is presentation metadata only.

## GTK CSS artifact

Nixling also writes `/etc/nixling/ui-colors.css` by default. It exposes
GTK-compatible `@define-color` declarations for bars, launchers, and
compositor-adjacent stylesheets:

```css
@import url("/etc/nixling/ui-colors.css");

#work {
  border-left-color: @nixling_env_work_accent;
}
```

Color definitions use underscores so GTK/Waybar parsers accept them.
Definition names include:

| Definition | Meaning |
| --- | --- |
| `nixling_host_accent` | Host identity accent |
| `nixling_state_running` | Running state accent |
| `nixling_state_transitioning` | Starting/stopping state accent |
| `nixling_state_pending_restart` | Pending-restart state accent |
| `nixling_state_error` | Error state accent |
| `nixling_state_denied` | Authorization-denied state accent |
| `nixling_state_unknown` | Unknown/unavailable state accent |
| `nixling_env_<env>_accent` | Environment identity accent |
| `nixling_vm_<vm>_border_active` | VM active border color |
| `nixling_vm_<vm>_border_inactive` | VM inactive border color |
| `nixling_vm_<vm>_border_urgent` | VM urgent border color |

Hyphens in environment and VM names are emitted as underscores in the CSS
definition name, for example VM `work-aad` becomes
`nixling_vm_work_aad_border_active`.

## Niri backend

Enable niri output with:

```nix
nixling.site.ui.compositors.niri.enable = true;
```

Nixling writes `/etc/nixling/niri-vm-borders.kdl` by default. The KDL
renders `active-color`, `inactive-color`, and `urgent-color` from the same
resolved VM border model used by the JSON and GTK CSS artifacts.

The legacy `nixling.site.niriVmBorders` and `nixling.vms.<vm>.graphics.niriBorderColor`
options remain compatibility inputs for one release, but new
configuration should use the `nixling.site.ui.*`,
`nixling.envs.<env>.ui.*`, and `nixling.vms.<vm>.ui.*` paths.
