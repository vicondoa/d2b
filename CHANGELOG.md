# Changelog

All notable changes to nixling are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Pre-1.0 minor releases may break public APIs. When practical,
deprecations ship one minor release before removal.

## Unreleased

## [0.1.0] - 2026-05-19

First public alpha release.

**Audience:** single-user NixOS desktop wanting isolated workspaces
(work / personal / risky-dev) on one host. Wayland-native.

**Stable in v0.1.0:**

- `nixosModules.default` (host integration)
- `templates.default` (`nix flake init -t github:vicondoa/nixling`)
- `flake.checks.<sys>.eval-{minimal,multi-env,template,graphics}`
- `nixling@<vm>.service` lifecycle wrapper + the eight `nixling` CLI
  verbs (`up`, `down`, `status`, `list`, `switch`, `build`, `boot`,
  `test`, `rollback`, `generations`, `gc`, `audio`, `usb`, `console`,
  `keys`)
- `manifestVersion = 1` JSON contract (`/run/current-system/sw/share/nixling/vms.json`)
- VM-name regex `^[a-z][a-z0-9-]*$`, reserved prefixes `sys-` and
  exact name `launcher`
- Per-env isolated network (auto-declared `sys-<env>-net` net VM,
  point-to-point uplink, isolated LAN bridge, dnsmasq, nftables NAT)
- Per-VM `/nix/store` hardlink farm
- Nixling-managed SSH keys
- Components: `graphics`, `tpm`, `usbip`, `audio`, `home-manager`

**Composition:** Sibling flake [`vicondoa/nixos-entra-id`][nei] (also
v0.1.0) provides Entra ID device-join via the per-VM
`nixling.vms.<vm>.config.imports = [ inputs.nixos-entra-id.nixosModules.default ]`
seam.

[nei]: https://github.com/vicondoa/nixos-entra-id

> Maintainer GitHub metadata reminder (apply on the GitHub UI, not in git):
>
> - **Description:** "NixOS microVM framework with isolated per-env
>   networking, Wayland/audio/USBIP/TPM components, and a
>   `nix flake init` template scaffold."
> - **Topics:** `nixos`, `nix-flake`, `microvm`, `wayland`,
>   `microvm-nix`, `nixos-template`, `entra-id`.



### Added (W6)

- `flake.checks.<system>.eval-{minimal,multi-env,template,graphics}` —
  the root flake now gates the example flakes + the template
  scaffold. The `graphics` check is x86_64-only.
- `tests/static.sh` now iterates `examples/*/flake.nix` running
  `nix flake check --no-build --all-systems` on each.
- `SECURITY.md` — disclosure path (GitHub Security Advisory primary;
  email fallback) plus the v0.1.0 alpha support matrix.
- `docs/explanation/design.md` — full threat model + defenses-in-depth
  list + a *Why not X* rationale FAQ (~823 LOC).
- `docs/how-to/migrating-from-microvm.md` — option mapping +
  step-by-step migration procedure + troubleshooting. Ordering is
  now build-before-state-move per W6 followup H1.
- Five per-component reference docs under
  `docs/reference/components-*.md` (graphics, tpm, usbip, audio,
  home-manager).
- `docs/reference/manifest-schema.{md,json}` polished with a rendered
  example payload generated from `tests/smoke-eval.nix`.

### Fixed (W6)

- `tests/{static,nixling-store,audio,lib}.sh` no longer assume
  `ROOT=/etc/nixos`; the value is derived from the script's own path
  so the suite runs from any clone. Spec correction #28 closed.
- `tests/nixling-store.sh:33` SC2157 (preexisting).
- Host-specific `NL_FILES` entries (`vms/personal-dev.nix`,
  `vms/work-aad.nix`) dropped or guarded so the static gate stays
  useful for the public flake.
- `tests/audio.sh` `NL_WAYLAND_USER` resolution chain genericized
  (no longer hardcoded to the maintainer's host user).
- README polish: `microVM` is defined inline on first use; a
  maintainer-anecdote phrasing was replaced with neutral wording;
  an encrypted-backup callout was added for `/var/lib/nixling/`.
- Manifest schema `manifestVersion` tightened from `minimum: 1` to
  `const: 1` so the JSON Schema matches the documented prose.

### Changed (W6)

- `docs/README.md` IA now reflects the shipping how-to and
  explanation docs (was previously reference-only).

### Added (W5)

- **`examples/minimal/`** — headless starter example: one env, one
  workload VM, ~25-line flake. The "is nixling for me?" sanity
  test.
- **`examples/graphics-workstation/`** — desktop VM with
  `graphics.enable`, `audio.enable`, and `usbip.yubikey` all on.
  Exercises every host-side sidecar component.
- **`examples/multi-env/`** — two parallel `nixling.envs.<env>`
  instances (work + personal) demonstrating per-env LAN
  isolation, per-env net VMs, per-env USBIP backends, and the
  route-preflight fail-closed gate.
- **`examples/with-entra-id/`** — composition with the sibling
  [`vicondoa/nixos-entra-id`][nixos-entra-id] flake; shows how
  the two trees meet at `nixling.vms.<vm>.config.imports`
  without either flake depending on the other.
- **`templates/default/`** — `nix flake init` scaffold with
  seven numbered `TODO:` markers and a matching
  `assertions = [ … ]` block. `nix flake check` on an un-edited
  scaffold fails with actionable messages until each sentinel is
  replaced.
- **`flake.templates.default`** — wires the template above so
  consumers can `nix flake init -t github:vicondoa/nixling`.

[nixos-entra-id]: https://github.com/vicondoa/nixos-entra-id

### Fixed (W5)

- **`nixos-modules/net.nix`:** neutralize base.nix's catch-all
  `10-eth-dhcp` systemd-networkd network on per-env net VMs. The
  catch-all (`matchConfig.Type = "ether"`) sorted lex-first
  against the per-MAC `10-uplink`/`10-lan` definitions and
  DHCP'd both NICs, preempting the static config. Now overridden
  via `lib.mkForce` with a sentinel MAC that never matches.
  Workload VMs are unaffected — they still inherit the base.nix
  DHCP fallback.
- **`nixos-modules/manifest.nix`:** dropped the redundant
  `default = { }` on the readOnly `nixling.manifest` option.
  The nixpkgs module system treats `default` as an extra
  definition; combined with `readOnly = true` and the matching
  `config.nixling.manifest = …` assignment, it produced
  "set multiple times" only when a graphics VM was synthesized.
  See `tests/smoke-eval-graphics.nix` for the regression test.

### Changed (W5)

- **README:** restructured to lead with a "Where to start" table
  pointing at the four examples and the template, and rewrote
  the Quick start to walk through the template path; the prior
  manual paste-in walkthrough is preserved under
  "Manual integration (without the template)".
- **`docs/README.md`:** added a Tutorials/Examples section
  linking the examples and the template; previously the docs
  index only mentioned the reference quadrant.

### Known gaps (deferred to v0.2.0 or Phase 7/8)

- **USBIP per-env units materialise even when no VM opts in.** Each
  `nixling.envs.<env>` declares `nixling-sys-<env>-usbipd-backend.service`
  and the corresponding proxy socket regardless of whether any
  workload VM in the env has `usbip.yubikey = true`. The units are
  idle when nothing opts in, but they are still installed (per-env
  baseline plumbing was intentional in W2; the unconditional
  materialisation is the gap). Tracked for v0.2.0; the relevant
  conditional belongs around `nixos-modules/network.nix:484-650`.
- **No static lint for `mkOption { default = …; readOnly = true; }`
  + matching `config.<…>` assignment.** Spec correction #29 was
  caught by the W5 reviewer panel, not by tooling. A Phase 7a
  follow-up will add a grep-level lint to prevent the
  `default + readOnly + config-assignment` trio from re-appearing.
  Trio detection is necessary because `store.nix` legitimately
  carries `readOnly + default` on options that have NO matching
  `config.<…>` assignment, so a two-of-three match is fine; only
  the full three is a bug.
- **Per-example flake-check loop is not fully hermetic for
  `examples/with-entra-id`.** `tests/static.sh` iterates
  `examples/*/flake.nix` and runs `nix flake check --no-build
  --all-systems` per example, but `with-entra-id` depends on the
  sibling `vicondoa/nixos-entra-id` flake which the core flake
  cannot pull in as an input. The example's own flake.lock pins
  the sibling and the iteration step exercises eval through it,
  but a clean-tree CI run cannot fully isolate the eval graph
  from the sibling. Tracked for v0.2.0.
- **VM-to-VM east-west traffic within the same env is not
  supported.** Workload taps on the per-env LAN bridge are
  configured with `Isolated = true`, so two workload VMs sharing
  `nixling.envs.<env>` can each reach the net VM (and via NAT,
  the upstream LAN) but cannot directly reach each other.
  Documented in `docs/explanation/design.md` and the
  `nixling.hostLanCidrs` option text. A future opt-in
  (e.g. `nixling.envs.<env>.intraLanIsolation = false`) is on the
  v0.2.0 wishlist.

### Added (W4)

- **Manifest contract is now a documented, versioned interface.**
  - `nixos-modules/manifest.nix` — typed `config.nixling.manifest`
    `attrsOf submodule` option. Replaces the inline manifest
    construction previously folded into `cli.nix`. The Nix module
    system catches schema regressions at eval time.
  - `docs/reference/manifest-schema.md` + `docs/reference/manifest-schema.json`
    (JSON Schema Draft 2020-12) — the v1 public manifest contract
    for downstream consumers (e.g. the future Rust CLI port). The
    JSON Schema is the canonical type spec; the prose doc is a
    field-by-field walkthrough + compatibility policy.
  - `docs/reference/cli-contract.md` — behavioural contract for any
    `nixling` CLI implementation (lifecycle FSM, signal semantics,
    exit codes, JSON vs human output, what is/is-not in scope).
  - `nixling.site.flakePath` is now derived as the CLI's default
    flake reference when unset (cli.nix lifecycle subcommands).
- **`docs/README.md`** — Diataxis IA index (tutorials, how-to,
  reference, explanation). Only the reference quadrant has content
  in W4; the others land on the path to v0.1.0.
- **Multi-arch eval coverage.** `tests/smoke-eval-aarch64.nix` —
  cross-evaluates a headless workload VM on `aarch64-linux`,
  verifying the eval graph stays multi-arch clean. Runtime is still
  `x86_64-linux`-only (cloud-hypervisor + crosvm); aarch64 is
  eval-coverage only.
- **Manifest validation gate.** `tests/static.sh` now renders the
  smoke manifest and runs a 5-check sequence against
  `docs/reference/manifest-schema.json`: render → parse schema →
  JSON-Schema validate → schema-side field cross-check →
  `manifestVersion >= 1`. Plus (W4-followup) a 6th check that diffs
  the prose schema's Per-VM-entry table against the JSON Schema's
  `properties` keys to catch md ↔ json drift.

### Changed (W4)

- **BREAKING for manifest consumers (pre-v0.1.0):** `manifestVersion`
  bumped `0 → 1`. The schema is now the documented contract. Future
  schema changes follow SemVer: minor field additions are
  backward-compatible; breaking changes bump the major (`2`, `3`,
  …). Consumers MUST refuse manifests with a newer major version
  than they were built against.
- **`nixling.vms.<vm>.graphics.enable` and
  `nixling.vms.<vm>.audio.enable` now refuse to evaluate on
  `aarch64-linux`** at the `microvm.vms` translation point. The
  eval-time error explains the constraint. Headless workload VMs
  (`graphics.enable = false; audio.enable = false;`) DO evaluate on
  aarch64-linux for cross-eval testing. Actual runtime is still
  x86_64-linux-only — the aarch64 path is eval-coverage only.
- `pkgs/{crosvm-patched,crosvm-seccomp,vhost-device-sound}/default.nix`
  now carry `meta.platforms = [ "x86_64-linux" ]`.
  `pkgs/spectrum-ch/default.nix` deliberately omits this (see
  in-file comment).
- `nixos-modules/options.nix` (internal refactor, no consumer-
  visible change): split into `options.nix` (aggregator) +
  `options-site.nix` + `options-envs.nix` + `options-vms.nix` for
  reviewability. The smoke-eval drvPath is bit-identical pre/post
  the split.

### Changed (W4-followup)

- **BREAKING for manifest consumers, security fix:** `sshKeyPath`
  removed from the per-VM JSON manifest. The W4 security reviewer
  flagged the field as a private-key path leak — the manifest at
  `/run/current-system/sw/share/nixling/vms.json` is world-readable,
  so exposing a per-VM private-key path leaks the location of
  secret material to every local user. The CLI now resolves the
  private-key path locally at Nix-eval time from
  `nixling.site.keysDir` (or per-VM `ssh.keyPath` override) and
  bakes a static per-VM mapping into the shell wrapper. Consumers
  reimplementing the CLI should mirror that: read
  `nixling.site.keysDir` from their own privileged config access,
  not from this world-readable file. The PUBLIC key path is not
  currently exposed; if a use case warrants it, a future
  `sshPubKeyPath` field is the recommended addition. `manifestVersion`
  stays at `1` — the schema was published moments ago in W4 and no
  external consumers exist yet, so this is a free pre-v0.1.0 break.
- `docs/reference/manifest-schema.json`: `manifestVersion.minimum`
  raised from `0` to `1`. The schema is the contract for v1+;
  pre-v1 manifests (the W2-followup stub) are no longer valid under
  this schema. (test-Med)
- `docs/reference/cli-contract.md`: subcommand inventory reconciled
  with `nixling --help`. `audit` now correctly documents the
  `--strict` + `--human` flags (`--human` auto-enables on TTY);
  `rotate-known-host <vm>` (the companion to `trust`) added to the
  subcommand table and to the human/JSON output section.
- `docs/reference/cli-contract.md`: "What is NOT in this contract"
  section expanded. Spells out that microvm.nix internal lifecycle,
  swtpm internals, virtiofsd implementation, and polkit grant
  specifics are framework-internal; and draws the line between
  contract-bound unit names (`nixling@<vm>.service`,
  `microvm@<vm>.service`) and framework-internal unit names
  (sidecars, USBIP proxies — these MUST be read from the manifest's
  `audioService` etc. fields, not hardcoded).
- `tests/static.sh`: `nix flake check` now uses `--all-systems` so
  Layer-1 exercises both x86_64-linux and aarch64-linux flake
  outputs, not just the builder's system. (sw-arch-Med)
- `tests/static.sh`: 6th manifest-contract check added — diffs the
  field-name column of the prose Per-VM-entry table in
  `docs/reference/manifest-schema.md` against the JSON Schema's
  `$defs.vmEntry.properties` keys, failing the gate if either side
  has a field the other doesn't. (sw-arch-Med)
- README: project status now states runtime is tested on
  `x86_64-linux` desktop and eval-tested for headless
  `aarch64-linux` (reflects the W4 cross-eval coverage).
  (product+docs-Med)
- README: documentation section replaces "docs will live under
  docs/" with direct bullets pointing at the manifest schema and
  CLI contract under `docs/reference/`. (product+docs-Med)
- `tests/README.md`: refreshed for the W4 additions —
  `manifestVersion = 1`, 10/10 assertions-eval cases, the 6-step
  manifest-contract gate (including the new md/json drift detection),
  and the multi-arch eval coverage. (docs-Med)

### Reorganised (W4-followup)

- Diataxis reorg. `docs/manifest-schema.{md,json}` →
  `docs/reference/manifest-schema.{md,json}`; `docs/cli-contract.md`
  → `docs/reference/cli-contract.md`. Added `docs/README.md` as the
  IA index. (docs-Med). All path references in
  `nixos-modules/manifest.nix`, `tests/static.sh`, and the moved
  docs' cross-links updated.

### Removed

- (W3b/Phase-2b) **`nixling.vms.<vm>.entra-id.*` option removed.**
  Himmelblau / Microsoft Entra ID support has moved out of the
  nixling framework and into the sibling `vicondoa/nixos-entra-id`
  flake. To migrate, add the flake as an input and import its
  module into the VM's guest config:

  ```nix
  inputs.nixos-entra-id.url = "github:vicondoa/nixos-entra-id";

  nixling.vms.<vm>.config.imports = [
    inputs.nixos-entra-id.nixosModules.default
  ];

  # Move each `nixling.vms.<vm>.entra-id.<key>` into the guest
  # config; see the nixos-entra-id README for the new schema.
  ```

  The `nixling.vms.<vm>.entra-id` attribute is kept as a hidden
  stub option so leftover assignments produce a readable
  assertion error (with migration instructions) instead of a
  cryptic "option does not exist" message from the module
  system. Final removal of the stub is tracked for v0.2.0.

- (W3b/Phase-2b) Three host-side activation scripts removed from
  `nixos-modules/host-activation.nix`:
  - **`nixlingSbctlBackup`** — moved maintainer-specific
    `*-backup.tar.gz` files from `$HOME` into `/var/lib/sbctl/backup/`.
    Not a framework concern. Consumers who relied on this should
    handle their own backup-file relocation outside nixling.
  - **`nixlingStoreChownRepair`** — one-shot repair for a past chown
    bug (an earlier `modules/nixling/store.nix` revision leaked
    `group=kvm` into `/nix/store` inodes via the per-VM hardlink
    farm). New installs are unaffected. Consumers upgrading from a
    pre-public nixling that ran with the buggy revision should run
    the historical repair script from `/etc/nixos` once and then
    drop the activation script there; the bug cannot recur on
    Phase-2b and later code.
  - **`nixlingMigrateState`** — one-shot renamer
    (`/var/lib/microvms/<vm>` → `/var/lib/nixling/vms/<vm>`, plus
    `/var/lib/swtpm/<vm>` → `vms/<vm>/swtpm/`). New installs land
    directly on the Phase-2a layout. Pre-public consumers should
    use the Phase 9 migration script (or perform the moves manually
    following the same logic) before switching to the public flake.

  These deletions remove all host-specific bias from the public
  framework's activation phase. The remaining two activation
  scripts (`nixlingVmStatePerms`, `nixlingNetVmVarImgPerms`,
  formerly `nixlingRouterVarImgPerms`) only adjust file ownership
  on per-VM disk images and contain no host-specific assumptions.

### Changed (W3b)

- **`nixling.vms.<vm>.ssh.keyPath` is NOT removed.** Earlier W3b
  phase-2b commit messages claimed otherwise; that was a mis-
  description of the change. The option still exists. What changed
  is its effective default: when left unset (`null`), the CLI now
  derives the SSH-key path from `nixling.site.keysDir` as
  `<keysDir>/<vm>_ed25519`, matching the framework-managed Ed25519
  key generated by `host-keys.nix` on every activation. Consumers
  who explicitly set a path still win; the option's `null` default
  just means "let the framework pick". This makes the
  framework-managed key the zero-config happy path while keeping
  the option-shape stable for consumers supplying their own keys
  (e.g. a hardware-backed Yubikey-resident key).

- (W3b/2026-05-19) Net VM `users.allowNoPasswordLogin` is set to
  `lib.mkDefault true`. Net VMs receive SSH keys via runtime
  injection (`nixling-load-host-keys.service` reads
  `<stateDir>/vms/<vm>/host-keys/` over virtiofs); they have no
  eval-time authorized_keys. Without the flag, NixOS module-eval
  fires the `users.allowNoPasswordLogin` assertion before runtime
  injection runs. Sealed-appliance consumers can override with
  `mkForce`.
- (W3b/2026-05-19) GPU sidecar (`nixling-<vm>-gpu.service`)
  hardening tightened: `NoNewPrivileges`, `ProtectSystem=strict`,
  `PrivateTmp`, `ProtectHome`, `DevicePolicy=closed` with a
  `/dev/kvm` + render-node allowlist, `RestrictAddressFamilies =
  [ AF_UNIX AF_NETLINK AF_VSOCK ]`,
  `SystemCallArchitectures=native`, narrow `ReadWritePaths`.
  Two omissions documented in source comments:
  `MemoryDenyWriteExecute` (crosvm GPU JIT triggers SIGSYS) and
  `AF_VSOCK` retained (cloud-hypervisor sd_notify path).
- (W3b/2026-05-19) IPv6 disabled on workload + net VM guest
  networkd (`LinkLocalAddressing=no`, `IPv6AcceptRA=false`); net
  VM nft rules DROP `ip6` forward. Net stack is IPv4-only by
  construction.
- (W3b/2026-05-19) Route preflight oneshot
  (`nixling-net-route-preflight.service`) now FAILS CLOSED on
  conflict — exit 1 on any env-vs-route mismatch instead of
  WARN+exit 0. `RemainAfterExit=true`, `Before=` each enabled
  nixling-managed VM unit, `RequiredBy=` each wrapper, so a stale
  host route blocks VM start until the operator clears it. (W3b
  H1 followup.)
- (W3b/2026-05-19) Inter-env CIDR overlap check now performs real
  IPv4 prefix arithmetic (`lib.cidrOverlaps` in
  `nixos-modules/lib.nix`) instead of exact-string equality.
  Containment (e.g. `10.0.0.0/16` ⊃ `10.0.1.0/24`) is rejected.
  Env-vs-`hostLanCidrs` is checked under the same helper. (W3b H3
  followup.)
- (W3b/2026-05-19) `nixling.site.yubikey.enable = false` actually
  gates the host-side udev rules + `usbip-host` kernel module.
  Previous phase-2b commit declared the option but never read it.
  (W3b H4 followup.)
- (W3b/2026-05-19) `nixling keys rotate <vm>` now scrubs the OLD
  pubkey from the guest's `~/.ssh/authorized_keys` (matched by
  SHA256 fingerprint) AFTER the new key is verified — rotation
  used to leave the old key authorized forever. Retention
  bounded: 3 most recent generations under
  `<keysDir>/old/<ts>/`; older are pruned post-rotation. Help
  text updated. (W3b H5 followup.)

### Added (W3b)

- **`nixling.site.*` public option surface.** Site-specific knobs
  extracted from previously-hardcoded references to the
  maintainer's host setup. Every option is opt-in; defaults give a
  fully headless framework with no Wayland integration. Public
  options:
  - `nixling.site.stateDir` — root of every nixling-managed state
    file (default `/var/lib/nixling`). **Advisory only in v0.1.0**
    (see option description); full threading lands in v0.2.0.
  - `nixling.site.keysDir` — directory for framework-managed
    per-VM SSH keys (default `${stateDir}/keys`). Same advisory
    caveat for v0.1.0.
  - `nixling.site.waylandUser` — primary Wayland user; required
    for any VM with `graphics.enable = true` or `audio.enable =
    true`.
  - `nixling.site.launcherUsers` — users added to the
    `nixling-launcher` group (polkit grant for VM start/stop).
  - `nixling.site.userAuthorizedKeys` — global authorized SSH
    keys merged into every VM at boot. Validated at eval time
    against an allowlist of supported key types; private-key
    markers rejected.
  - `nixling.site.yubikey.enable` — host-side Yubico udev rules +
    `usbip-host` kernel module. Default true.
  - `nixling.site.flakePath` — default flake reference for the
    `nixling` CLI's lifecycle subcommands (`build`, `switch`,
    `boot`, `test`). Nullable.
- **`nixling.vms.<vm>.userAuthorizedKeys`** — per-VM
  authorized SSH keys, merged with `site.userAuthorizedKeys`.
- **`nixling.audio.users`** — host-side option propagating an
  audio-group membership list into the guest. Default falls back
  to `[ vm.ssh.user ]` when unset.
- **Framework-managed per-VM SSH keys.** Activation
  (`nixos-modules/host-keys.nix`) generates an Ed25519 keypair
  per enabled VM under `<keysDir>/<vm>_ed25519`. Atomic via
  staging + `mv -T`; protected by `flock` on `<keysDir>/.lock`.
  The pubkey is staged under
  `<stateDir>/vms/<vm>/host-keys/host.pub` and injected into the
  guest at boot via virtiofs.
- **`nixling keys` CLI subcommands.**
  - `nixling keys list [--json]` — fingerprint + path + mtime
    per VM.
  - `nixling keys show <vm>` — print the pubkey.
  - `nixling keys rotate <vm>` — atomic rotate-and-verify with
    SHA256-fingerprint-based old-key scrub + 3-generation
    retention (see Changed entry above).
- **`nixling-load-host-keys.service`** (guest-side) — fail-closed
  service that reads `/run/nixling-host-keys/` and writes the
  union of `host.pub` + user-authorized-keys into the SSH user's
  `~/.ssh/authorized_keys`.
- **`scripts/migrate-nixling-v0.1.0.sh`** (W3b H6) — one-shot host
  migration script for consumers upgrading from a pre-public
  in-tree nixling layout. Preserves TPM state byte-for-byte.
  Has `--dry-run` and `--rollback`. Committed under
  `scripts/` so CI can shellcheck it.
- **`tests/smoke-eval.nix`** (W3b H9) — minimal consumer-style
  nixosSystem that imports `nixling.nixosModules.default` and
  exercises the eval graph end-to-end. Wired into
  `tests/static.sh` Layer-1.
- **`tests/assertions-eval.sh`** (W3b H10) — 8 regression tests
  exercising every eval-time invariant in the schema (CIDR shape,
  CIDR overlap, key validation, `waylandUser` presence, …).
- **`nixos-modules/lib.nix#cidrOverlaps`** — pure-Nix IPv4 prefix
  overlap helper used by network.nix assertions. Same file gains
  `parseCidr` as a public helper.

### Added

- (W0/2026-05-18) Initial flake skeleton with Apache-2.0 license,
  `x86_64-linux` + `aarch64-linux` eval, `microvm.nix` input, and
  reserved-but-inert `nixosModules.default`.
- (W1/2026-05-18) Mechanical lift of nixling modules from
  `/etc/nixos/modules/nixling/` into the public flake:
  - 9 core modules under `nixos-modules/` (`default`, `options`,
    `lib`, `host`, `network`, `base`, `store`, `cli`;
    `router.nix` renamed to `net.nix`);
  - 6 component modules under `nixos-modules/components/`
    (`graphics`, `tpm`, `usbip`, `home-manager`; `audio` split into
    `audio/{guest,host}.nix`);
  - Extracted pkgs: `spectrum-ch`, `vhost-device-sound`,
    `crosvm-patched`, `crosvm-seccomp`, `patches`;
  - 6 generic test scripts under `tests/`.
- (W2/2026-05-18) `systemd.services."nixling@"` wrapper template with
  explicit `ExecStart` / `ExecStop` / `PropagatesStopTo` (planning-round
  Critical #1 — `BindsTo` alone does not propagate stops).
- (W2/2026-05-18) Eval-time assertions for VM names
  (`^[a-z0-9][a-z0-9-]*$`, no `sys-` prefix, not `launcher`) and env
  names (≤ 8 chars).
- (W2/2026-05-18) `nixos-modules/assertions.nix` as a dedicated
  assertions module.
- (W2-followup/2026-05-18) Top-level `manifestVersion = 0` stub field
  in the per-VM JSON manifest (Phase 5 bumps to 1). Stashed under
  the reserved `_manifest` sentinel key; user-declared VM names
  cannot start with `_` per the W2-followup H1 stricter regex.

### Changed

- (W2/2026-05-18) **BREAKING.** Option namespace renamed:
  - `nixling.networks.<env>` → `nixling.envs.<env>`;
  - `nixling.networks.<env>.routerName` →
    `nixling.envs.<env>.netName`;
  - `nixling.networks.<env>.extraRouterConfig` →
    `nixling.envs.<env>.extraNetConfig`.
- (W2/2026-05-18) **BREAKING.** Per-env auto-declared VM renamed:
  `<env>-router` → `sys-<env>-net`.
- (W2/2026-05-18) **BREAKING.** Systemd unit naming convention:
  - `swtpm@<vm>` → `nixling-<vm>-swtpm`;
  - `nixling-snd@<vm>` → `nixling-<vm>-snd`;
  - `nixling-gpu-<vm>` → `nixling-<vm>-gpu`;
  - `nixling-store-sync@<vm>` → `nixling-<vm>-store-sync`;
  - `usbipd-nixling` → `nixling-sys-usbipd`;
  - `usbipd-nixling-<env>` → `nixling-sys-<env>-usbipd-proxy`.
- (W2/2026-05-18) **BREAKING.** System users/groups renamed:
  `nixling-gpu-<vm>` → `nixling-<vm>-gpu`, `nixling-snd-<vm>` →
  `nixling-<vm>-snd`, `swtpm-<vm>` → `nixling-<vm>-swtpm`.
- (W2/2026-05-18) **BREAKING.** State-dir layout:
  - `<stateDir>/<vm>/` → `<stateDir>/vms/<vm>/`;
  - `<stateDir>/<env>-router/` → `<stateDir>/vms/sys-<env>-net/`;
  - `<stateDir>/swtpm/<vm>/` → `<stateDir>/vms/<vm>/swtpm/`;
  - `/run/nixling-snd/<vm>/snd.sock` →
    `/run/nixling/vms/<vm>/snd.sock`.
- (W2/2026-05-18) **BREAKING.** Manifest JSON contract: `isRouter` →
  `isNetVm`, `routerVm` → `netVm`. Top-level `manifestVersion = 0`
  added in W2-followup; Phase 5 will bump.
- (W2-followup/2026-05-18) **BREAKING.** VM/env name regex tightened
  from `^[a-z0-9][a-z0-9-]*$` to `^[a-z][a-z0-9-]*$` (require leading
  letter). Matches systemd-escape best practices; avoids ambiguity
  with tooling that treats a leading digit as a numeric index
  (`ip link show 42web-l10`). No existing in-tree names trip the
  stricter rule; consumers with numeric-prefixed VM/env names must
  rename.
- (W2-followup/2026-05-18) CLI: `nixling up/down/status` now target
  `nixling@<vm>.service` (the user-facing wrapper) instead of
  `microvm@<vm>.service` directly. Lifecycle propagates via the
  wrapper's BindsTo / ExecStop. Diagnostic flows
  (`status --verbose`, `journalctl` examples) keep their
  `microvm@<vm>` references but label them "backend" /
  "implementation detail".
- (W2-followup/2026-05-18) CLI: `nixling list` / `nixling status`
  output tag for system VMs changed from `(router)` to `(net-vm)`.
  Helper renames: `ensure_router_up` → `ensure_net_vm_up`,
  `router_active` → `net_vm_active`, `IS_ROUTER` → `IS_NET_VM`.
  User-facing prose `router` / `router VM` → `net` / `net VM` (kept
  `routing/routes` only where describing the network function).
- (W2-followup/2026-05-18) `nixling-launcher` polkit grant tightened
  to an exact-unit allowlist generated at NixOS eval time from
  `cfg.vms` + `cfg.envs`, restricted to `start` / `stop` / `restart`
  verbs only. Drops the bare `microvm@*` prefix wildcard; default-
  deny invariant restored. Recovery / debugging paths can still
  authenticate via sudo or polkit-prompt.

### Notes

- Pre-v0.1.0 — breaking changes do not get a deprecation period.
  There is no compat shim for the old `nixling.networks` namespace
  or for any of the renamed unit / user / state-dir identifiers.
- The first tagged release will be `v0.1.0`. Until then, treat
  `main` as unstable.
- v0.1.0 will ship in lockstep with
  [`vicondoa/nixos-entra-id`][nixos-entra-id] v0.1.0; consumers
  using both should pin matching tags.

[nixos-entra-id]: https://github.com/vicondoa/nixos-entra-id
