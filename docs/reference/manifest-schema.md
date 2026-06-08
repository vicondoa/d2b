# nixling JSON manifest schema

**Status:** stable from `manifestVersion = 1` onward (Phase 5).
**Source of truth:** [`manifest-schema.json`](./manifest-schema.json)
(JSON Schema Draft 2020-12). When this document and the JSON Schema
disagree, the JSON Schema wins.

## What this is

`nixling` evaluates a typed per-VM manifest at Nix-evaluation time and
ships it as a single JSON file at:

```
/run/current-system/sw/share/nixling/vms.json
```

The current bash CLI consumes this file via `jq`. A future Rust port of
the CLI is the primary reason the schema exists as a documented
contract — it lets the port generate types via `serde_json` /
`schemars` and stay decoupled from the Nix module system.

Producer: `nixos-modules/manifest.nix` (declares
`config.nixling.manifest`, renders the JSON file via
`pkgs.writeTextFile`, registers the file with `environment.systemPackages`).

Consumer (today): the `nixling` shell wrapper baked by
`nixos-modules/cli.nix`.

Consumer (tomorrow): a Rust CLI living in
`packages/nixling-cli/` (Phase 8 / post-v0.1.0).

## Top-level structure

```jsonc
{
  "_manifest": { "manifestVersion": 1 },

  "<vm-name>": {
    "name":           "...",
    "graphics":       false,
    "tpm":            false,
    "usbipYubikey":   false,
    "audio":          false,
    "tap":            "...",
    "bridge":         "..." | null,
    "env":            "..." | null,
    "isNetVm":        false,
    "netVm":          "..." | null,
    "usbipdHostIp":   "..." | null,
    "stateDir":       "/var/lib/nixling/vms/<vm-name>",
    "apiSocket":      "/var/lib/nixling/vms/<vm-name>/<vm-name>.sock",
    "gpuSocket":      "/var/lib/nixling/vms/<vm-name>/<vm-name>-gpu.sock",
    "tpmSocket":      "/run/swtpm/<vm-name>/sock",
    "audioStateFile": "/var/lib/nixling/vms/<vm-name>/state/audio-state.json",
    "audioService":   "nixling-<vm-name>-snd.service",
    "staticIp":       "..." | null,
    "sshUser":        "..." | null
  },

  "<another-vm-name>": { ... }
}
```

Every top-level key is either:

1. **A reserved key** starting with `_` (currently only `_manifest`), or
2. **A VM name** matching `^[a-z][a-z0-9-]*$` (the regex enforced by
   `nixos-modules/assertions.nix`'s `vmNameOk`).

The leading-underscore rule means reserved keys can never collide with
a valid VM name, so the manifest can grow new sentinel fields without
ever needing a schema-version bump for that reason alone.

## Reserved keys

### `_manifest`

```jsonc
"_manifest": {
  "manifestVersion": 1   // unsigned integer
}
```

| Field             | Type             | Required | Description                                                                                  |
|-------------------|------------------|----------|----------------------------------------------------------------------------------------------|
| `manifestVersion` | unsigned integer | yes      | Schema version. Bumped on every breaking change. See "Compatibility policy" below.           |

Future reserved keys (none yet exist) would also start with `_`. CLI
implementations MUST silently ignore unknown top-level keys whose name
starts with `_`.

## Per-VM entry

The keys below are documented in declaration order from
`nixos-modules/manifest.nix`. The JSON Schema (`manifest-schema.json`)
is the canonical type spec; the table below is for human readability.

| Field            | Type               | Required | Notes                                                                                                                    |
|------------------|--------------------|----------|--------------------------------------------------------------------------------------------------------------------------|
| `name`           | string             | yes      | The VM's attribute key (`nixling.vms.<name>`). Matches the enclosing JSON key. Pattern `^[a-z][a-z0-9-]*$`.               |
| `graphics`       | boolean            | yes      | Mirror of `nixling.vms.<name>.graphics.enable`. The CLI uses this to pick the launch path.                                |
| `tpm`            | boolean            | yes      | Mirror of `nixling.vms.<name>.tpm.enable`.                                                                                |
| `usbipYubikey`   | boolean            | yes      | Mirror of `nixling.vms.<name>.usbip.yubikey`. `nixling usb <vm>` refuses to run when false.                               |
| `audio`          | boolean            | yes      | Mirror of `nixling.vms.<name>.audio.enable`. The *capability* bit. Live grant state is in `audioStateFile`.               |
| `tap`            | string             | yes      | Host-side tap-device name. Derived: `<env>-l<index>` (workload), `<env>-u2` (net VM), or `vm-<name>` (legacy).            |
| `bridge`         | string \| null     | yes      | Linux bridge the tap attaches to. Workload: `br-<env>-lan`. Net VM: `br-<env>-up`. Legacy hand-rolled VM: `null`.         |
| `env`            | string \| null     | yes      | Env this VM belongs to (workload) or serves (net VM). Null for legacy hand-rolled VMs.                                   |
| `isNetVm`        | boolean            | yes      | True iff this VM is the auto-generated `sys-<env>-net`. CLI uses this for bring-up ordering.                              |
| `netVm`          | string \| null     | yes      | For workload VMs: name of the net VM serving this VM's env. Null for net VMs and legacy VMs.                              |
| `usbipdHostIp`   | string \| null     | yes      | Host IP of the per-env usbipd proxy. `nixling usb <vm>` passes this to `usbip attach -r`. Null for net VMs/legacy.       |
| `stateDir`       | string             | yes      | Per-VM state dir. Currently hardcoded to `/var/lib/nixling/vms/<name>`. See `nixling.site.stateDir`'s advisory note.    |
| `apiSocket`      | string             | yes      | microvm.nix runner API socket (`<stateDir>/<name>.sock`). Used by `nixling down` for clean shutdown.                       |
| `gpuSocket`      | string             | yes      | crosvm-gpu sidecar control socket (`<stateDir>/<name>-gpu.sock`). Only meaningful when `graphics = true`.                  |
| `tpmSocket`      | string             | yes      | swtpm vTPM socket (`/run/swtpm/<name>/sock`). Only meaningful when `tpm = true`.                                          |
| `audioStateFile` | string             | yes      | Live audio-grant state file (`<stateDir>/state/audio-state.json`). `{ "mic": "on"\|"off", "speaker": "on"\|"off" }`.        |
| `audioService`   | string             | yes      | Host-side audio sidecar systemd unit (`nixling-<name>-snd.service`).                                                       |
| `staticIp`       | string \| null     | yes      | The VM's static LAN IP. Derived for env-attached VMs; null for legacy VMs with no `staticIp` set.                          |
| `sshUser`        | string \| null     | yes      | `nixling`-driven SSH username. Mirrors `nixling.vms.<name>.ssh.user`. Null for headless net VMs the CLI never SSH-attaches.|

### Field semantics — deep dive

#### `name` vs the enclosing key

The per-VM JSON object's enclosing key is the VM name. The `name`
field inside the object carries the same value. This redundancy is
intentional — it lets a Rust consumer parse a single per-VM entry
out of a stream without losing the name, and it makes the CLI's
`jq` filters that emit per-VM JSON to subprocesses self-describing.

A v2+ schema MAY drop `name` if every consumer is updated to derive
it from the enclosing key. Until then, treat `name` and the
enclosing key as required to match.

#### Why so many path fields are derivable

`stateDir`, `apiSocket`, `gpuSocket`, `tpmSocket`, `audioStateFile`,
and `audioService` are all mechanically derivable from `name` today.
The schema carries them explicitly because:

1. **The path layout is part of the framework's public contract.** A
   v2 schema might thread `nixling.site.stateDir` overrides through
   to these paths (currently advisory-only); when that happens, the
   manifest is the place to look up the resolved path, not a string
   template inside the CLI.
2. **The Rust CLI shouldn't have to know the template syntax.** The
   bash CLI today inlines `/var/lib/nixling/vms/$VM/$VM.sock` in
   half a dozen places. That's the kind of duplication the manifest
   is designed to centralise.

#### `staticIp` vs the legacy hand-rolled path

When `nixling.vms.<name>.env` is set (the recommended path),
`staticIp` is derived from `(env, index)`. When `env` is null, the
framework reads `nixling.vms.<name>.staticIp` directly. That option
is `deprecated` in `options.nix` — new code should always go
through `env` + `index`.

#### `sshUser` and the private-key path

`sshUser` is the only SSH coordinate carried in the public manifest.
The **private-key path is intentionally NOT in the manifest** (W4
followup, security): the manifest at
`/run/current-system/sw/share/nixling/vms.json` is world-readable, and
exposing a per-VM private-key path leaks the location of secret
material to every local user.

The CLI resolves the private-key path locally from
`config.nixling.site.keysDir` (or `nixling.vms.<name>.ssh.keyPath`
when the consumer overrode it) at Nix-eval time, then bakes the
per-VM mapping into the shell wrapper. Consumers reimplementing the
CLI should mirror that pattern:

1. Read `config.nixling.site.keysDir` from a separate root-only file
   (or derive it from the host's NixOS config the CLI already has
   privileged access to).
2. Compose `<keysDir>/<name>_ed25519` (the framework-managed key) as
   the default; let consumers override per VM via their own config.

If a future use case warrants a manifest-side hint, the recommended
addition is `sshPubKeyPath` (the `.pub` file under `<keysDir>/`).
Public keys are not secret; the private key is then derivable by
convention as the same path minus the `.pub` suffix.

## Compatibility policy

**`manifestVersion` is a single non-negative integer.**

| Change kind                                               | Bump | Rationale                                       |
|-----------------------------------------------------------|------|-------------------------------------------------|
| Add new optional per-VM field                             | no   | Old consumers ignore unknown keys.              |
| Add new reserved `_*` top-level key                       | no   | Reserved namespace exists for forward compat.   |
| Remove a per-VM field                                     | yes  | Old consumers may read missing key as null.     |
| Rename a per-VM field                                     | yes  | Old consumers won't find the renamed field.    |
| Narrow a field's type (e.g. `nullOr str` → `str`)         | no   | Strictly more permissive for consumers.         |
| Widen a field's type (e.g. `str` → `nullOr str`)          | yes  | Old consumers may not handle the new variant.   |
| Change a field's semantics without renaming it            | yes  | Old consumers do the wrong thing silently.      |
| Make a previously-optional field required                 | yes  | Old producers may not emit it.                  |
| Make a previously-required field optional                 | no   | Strictly more permissive.                       |

### What a CLI consuming `manifestVersion = N` MUST do when it sees `N+k`

For `k > 0`:

1. **REFUSE to operate on the manifest.** Print a clear error like
   `nixling: manifest version 2 is newer than this CLI build (1); upgrade the CLI`.
   Exit with status `4` (manifest-incompatible — see `cli-contract.md`).
2. **DO NOT attempt graceful degradation.** A breaking schema change
   means at least one field's type or semantics has shifted. Best-effort
   parsing risks corrupting state (e.g., a renamed `tap` field becoming
   "missing" and the CLI taking some default destructive action).

For `k = 0` (same version):

1. **Parse the manifest normally.**
2. **Tolerate unknown top-level reserved keys** (starting with `_`).
3. **Tolerate unknown per-VM fields.** (Future additive changes.)
4. **Tolerate unknown VM names** in the map — the consumer may not
   recognise every VM but should still be able to operate on the
   ones it knows about.

For `k < 0` (manifest is older):

This case happens when the user's system flake is older than their
installed CLI build (e.g. CLI installed via flake, system not
rebuilt yet). The CLI MAY operate but SHOULD warn:
`nixling: manifest version 0 is older than this CLI build (1); some fields may be missing — rebuild the system`.

The CLI MUST then handle missing fields gracefully (treating them as
null where the type permits, or refusing the specific subcommand
that needs them).

### Migration path

When `manifestVersion` is bumped, the producer (`manifest.nix`) and
the consumers (the bash `nixling` script in `cli.nix`, and the
Rust CLI) are updated in the same PR. The Rust CLI versions itself
independently and declares the highest manifest version it supports
in a CLI-side constant; users running an older CLI against a newer
manifest get the error above.

## Examples

### Minimal — rendered from `tests/smoke-eval.nix`

The smoke-eval test wires one env (`work`, LAN `10.20.0.0/24`, uplink
`192.0.2.0/30`) and one workload VM (`corp-vm`, `index = 10`, SSH user
`alice`). The framework auto-materialises the env's net VM
(`sys-work-net`). Running:

```bash
nix eval --json --impure --expr '
  let f = builtins.getFlake (toString ./.); n = f.inputs.nixpkgs.lib.nixosSystem;
      cfg = n { system = builtins.currentSystem; modules = [ /* see tests/smoke-eval.nix */ ]; };
  in builtins.fromJSON cfg.config.nixling._manifestPkg.text
' | jq .
```

produces exactly:

```json
{
  "_manifest": {
    "manifestVersion": 1
  },
  "corp-vm": {
    "apiSocket": "/var/lib/nixling/vms/corp-vm/corp-vm.sock",
    "audio": false,
    "audioService": "nixling-corp-vm-snd.service",
    "audioStateFile": "/var/lib/nixling/vms/corp-vm/state/audio-state.json",
    "bridge": "br-work-lan",
    "env": "work",
    "gpuSocket": "/var/lib/nixling/vms/corp-vm/corp-vm-gpu.sock",
    "graphics": false,
    "isNetVm": false,
    "name": "corp-vm",
    "netVm": "sys-work-net",
    "sshUser": "alice",
    "stateDir": "/var/lib/nixling/vms/corp-vm",
    "staticIp": "10.20.0.10",
    "tap": "work-l10",
    "tpm": false,
    "tpmSocket": "/run/swtpm/corp-vm/sock",
    "usbipYubikey": false,
    "usbipdHostIp": "192.0.2.1"
  },
  "sys-work-net": {
    "apiSocket": "/var/lib/nixling/vms/sys-work-net/sys-work-net.sock",
    "audio": false,
    "audioService": "nixling-sys-work-net-snd.service",
    "audioStateFile": "/var/lib/nixling/vms/sys-work-net/state/audio-state.json",
    "bridge": "br-work-up",
    "env": "work",
    "gpuSocket": "/var/lib/nixling/vms/sys-work-net/sys-work-net-gpu.sock",
    "graphics": false,
    "isNetVm": true,
    "name": "sys-work-net",
    "netVm": null,
    "sshUser": null,
    "stateDir": "/var/lib/nixling/vms/sys-work-net",
    "staticIp": "192.0.2.2",
    "tap": "work-u2",
    "tpm": false,
    "tpmSocket": "/run/swtpm/sys-work-net/sock",
    "usbipYubikey": false,
    "usbipdHostIp": null
  }
}
```

Notes on what to read out of this:

- Field order is alphabetical because the producer is
  `builtins.toJSON`, which sorts object keys. Consumers MUST NOT rely
  on ordering either way.
- The workload entry (`corp-vm`) carries `netVm = "sys-work-net"` and
  a non-null `usbipdHostIp` pointing at the env's host-side usbipd
  proxy.
- The net VM (`sys-work-net`) has `isNetVm = true`, `netVm = null`,
  `usbipdHostIp = null`, `sshUser = null`, `bridge = "br-work-up"`
  (the env's uplink bridge — not the LAN bridge), and `tap = "work-u2"`
  (the net VM's second uplink tap, conventionally the host-facing one).
- `_manifest.manifestVersion` is the schema-version sentinel; see
  "Compatibility policy".

### Graphics + audio + Yubikey

A workload VM with all the optional component bits flipped on:

```nix
nixling.vms.gui-vm = {
  enable = true;
  env = "work";
  index = 11;
  graphics.enable = true;
  audio.enable = true;
  usbip.yubikey = true;
  tpm.enable = true;
  ssh.user = "alice";
};
```

yields per-VM (the framework toggles the capability flags; path fields
are mechanically derived from `name` and the env):

```json
"gui-vm": {
  "apiSocket": "/var/lib/nixling/vms/gui-vm/gui-vm.sock",
  "audio": true,
  "audioService": "nixling-gui-vm-snd.service",
  "audioStateFile": "/var/lib/nixling/vms/gui-vm/state/audio-state.json",
  "bridge": "br-work-lan",
  "env": "work",
  "gpuSocket": "/var/lib/nixling/vms/gui-vm/gui-vm-gpu.sock",
  "graphics": true,
  "isNetVm": false,
  "name": "gui-vm",
  "netVm": "sys-work-net",
  "sshUser": "alice",
  "stateDir": "/var/lib/nixling/vms/gui-vm",
  "staticIp": "10.20.0.11",
  "tap": "work-l11",
  "tpm": true,
  "tpmSocket": "/run/swtpm/gui-vm/sock",
  "usbipYubikey": true,
  "usbipdHostIp": "192.0.2.1"
}
```

## Consuming the manifest

### From shell (bash)

```bash
MANIFEST=/run/current-system/sw/share/nixling/vms.json

# Enumerate VM names (skipping the _manifest sentinel):
jq -r 'to_entries[] | select(.key | startswith("_") | not) | .key' "$MANIFEST"

# Look up one VM's apiSocket:
jq -r --arg n corp-vm '.[$n].apiSocket' "$MANIFEST"

# Schema version check:
jq -r '._manifest.manifestVersion' "$MANIFEST"
```

### From Rust (future, sketch)

```rust
#[derive(Deserialize)]
struct Manifest {
    #[serde(rename = "_manifest")]
    meta: ManifestMeta,
    #[serde(flatten)]
    vms: HashMap<String, VmEntry>,
}

#[derive(Deserialize)]
struct ManifestMeta {
    #[serde(rename = "manifestVersion")]
    version: u32,
}

#[derive(Deserialize)]
struct VmEntry {
    name: String,
    graphics: bool,
    // ... see manifest-schema.json for the full field list
}

const SUPPORTED_VERSION: u32 = 1;
let m: Manifest = serde_json::from_str(&fs::read_to_string("/run/current-system/sw/share/nixling/vms.json")?)?;
if m.meta.version > SUPPORTED_VERSION {
    bail!("manifest version {} newer than CLI build ({}); upgrade the CLI",
          m.meta.version, SUPPORTED_VERSION);
}
```

## See also

- [`manifest-schema.json`](./manifest-schema.json) — formal JSON Schema.
- [`cli-contract.md`](./cli-contract.md) — the lifecycle / signal /
  exit-code contract the CLI must implement on top of this schema.
- `nixos-modules/manifest.nix` — the producer.
- `nixos-modules/cli.nix` — today's bash consumer.
