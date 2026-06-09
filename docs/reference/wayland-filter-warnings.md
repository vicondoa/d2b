# `graphics.waylandFilter` warning catalog

> **Reference** for the advisory warnings the nixling Wayland filter
> policy engine emits when operator configuration deviates from the
> baseline or touches a rule nixling classifies as required or
> high-risk.

> **Status:** planned option surface.  The Rust policy engine is present,
> but the NixOS option namespace
> `nixling.vms.<vm>.graphics.waylandFilter.*` is wired by the central
> graphics integration.  Until that wiring lands, the Nix snippets below
> document the intended surface and will fail if pasted into a host
> configuration.

Warnings are **advisory**: the NixOS configuration still evaluates
and builds when a warning condition is met once the option surface is
wired.  They are intended to surface in `nixos-rebuild switch` output and
in the `nixling down/up --apply` diagnostic stream.

Secure defaults emit **zero** `waylandFilter` warnings.  A clean
configuration with no overrides produces no output from this catalog once
the NixOS option surface is available.

## Warning conditions

The `W-*` names below are documentation anchors for the planned NixOS
warning surface.  The Rust policy engine currently emits human-readable
`PolicyWarning` messages; stable warning codes are added when the NixOS
option layer is wired.

### W-DENY-BASELINE

**Trigger:** An operator rule explicitly denies or sets `maxVersion`
below the usable minimum for a global that nixling classifies as a
required application-baseline global.

**Required baseline globals:** `wl_compositor`, `wl_shm`,
`xdg_wm_base`, `wl_seat`, `wl_output`.

**Example:**
```nix
nixling.vms.work.graphics.waylandFilter.rules = [
  { interface = "wl_compositor"; action = "deny"; reason = "test"; }
];
```

**Why it exists:** Denying these globals breaks ordinary guest
applications.  Most apps require compositor, shared-memory buffer,
XDG shell, input seat, and output objects to function at all.

**How to override intentionally:** Keep the rule and acknowledge the
warning in code comments.  The configuration is accepted; guest
apps on this VM may not render or receive input.

---

### W-DENY-ACCEL

**Trigger:** An operator rule disables or version-caps a dmabuf or
rendering global that nixling expects for GPU-accelerated graphics.

**Affected globals (examples):** `linux_dmabuf_v1`,
`wp_linux_drm_syncobj_manager_v1`, `zwp_linux_explicit_synchronization_v1`.

**Why it exists:** Disabling dmabuf/render globals causes guest apps
to fall back to software (llvmpipe) rendering, which significantly
reduces graphics performance and may break GPU-dependent apps.

**How to override intentionally:** Set the deny rule and accept the
performance regression.  The warning confirms that llvmpipe fallback
is expected.

---

### W-APP-ID-PREFIX

**Trigger:** `graphics.waylandFilter.appIdPrefix` is set to a value
other than `nixling.<vm>.` (including the empty string `""`).

**Why it exists:** The app-id prefix is what allows the host
compositor to identify which VM a window belongs to.  Changing or
removing it breaks the generated niri border rules
(`nixling.site.niriVmBorders`) and any compositor rules that rely on
the `nixling.<vm>.` namespace.

**How to override intentionally:** Set a non-default prefix and
manually update all compositor rules that match by app-id.  If you
set `appIdPrefix = ""`, no prefix is applied; guest app-ids are
forwarded unmodified, which makes VM identity disambiguation the
operator's responsibility.

---

### W-TITLE-PREFIX

**Trigger:** `graphics.waylandFilter.titlePrefix` is set to `""`.

**Why it exists:** The title prefix `[<vm>] ` provides VM
disambiguation in compositors that display window titles but do not
use app-ids for VM identification.  Removing it silently removes the
visual cue that a window belongs to a specific VM.

**How to override intentionally:** Set `titlePrefix = ""` and confirm
that your compositor provides alternative disambiguation (for example,
via niri workspace-per-VM rules that rely on app-id rather than
title).

---

### W-ENABLE-HIGH-RISK

**Trigger:** An operator enables a feature that is disabled by default
because it exposes a high-risk Wayland protocol surface.

**High-risk feature bundles:**

| Feature | Risk |
|---|---|
| `screen-capture` | Screen/image capture globals allow guest apps to capture the host display. |
| `virtual-input` | Virtual keyboard and pointer globals allow guest apps to inject arbitrary host input events. |
| `clipboard-control` | Privileged data-control globals allow guest apps to read or modify arbitrary host clipboard content. |
| `desktop-shell` | Layer-shell and privileged shell-surface globals give guest apps elevated compositor privileges. |
| `session-control` | Session lock, output power, output management, and workspace management globals give guest apps broad compositor control. |
| `security-context` | Wayland security-context extension is disabled until a concrete safe use case is identified. |

**Why it exists:** These protocols give guest apps abilities that
exceed ordinary window management.  Enabling them extends the trust
boundary from "guest app can render windows" to "guest app can capture
the screen, inject input, or lock the session on the host".

**How to override intentionally:** Enable the feature with an explicit
`reason` string in the feature config.  The warning documents that
the extension is active and the operator accepts the associated risk.
Treat any VM with these features enabled as a higher-trust guest and
review its isolation (Docker avoidance, `crossDomainTrusted`
justification).

---

### W-ALLOW-UNCLASSIFIED

**Trigger:** An operator adds an explicit `allow` rule for a global
that nixling has not yet classified as either a known-safe application
protocol or a known-high-risk protocol.

**Why it exists:** Unclassified globals may be safe or may expose
host-side privilege.  Nixling defaults to denying them until
classified.  This warning signals that the operator is taking
responsibility for the security posture of an unreviewed protocol.

**How to override intentionally:** Add the allow rule and document in
the `reason` field why the global is safe for this VM.  Consider
filing an issue or PR to have nixling classify the global so the
warning is resolved upstream.

---

### W-NIXLING-SECURITY-CRITICAL-DENY

**Trigger:** An operator allows a global that nixling marks
`nixlingSecurityCriticalDeny`.  This designation is reserved for
globals whose forwarding would directly violate a core nixling
security invariant (for example, a global that would allow a guest to
access raw host input devices or bypass the filter proxy entirely).

**Why it exists:** Unlike high-risk features (which are allowed with a
warning), security-critical-deny globals represent cases where
forwarding is inconsistent with the threat model for any guest.

**How to override intentionally:** This warning is emitted even when
the allow rule is accepted.  If a specific workload genuinely requires
a security-critical global, document the justification and threat
model deviation in a host configuration comment.  The configuration
still builds; the warning serves as a persistent audit trail.

---

## Warning vs. hard assertion

Warnings never become hard assertions.  Every warning condition still
produces a valid, buildable NixOS configuration.  The distinction from
a hard assertion (`lib.mkAssert` or `config.assertions`) is
intentional: operators may have valid workload-specific reasons to
deviate from nixling's baseline, and nixling should facilitate
informed, documented exceptions rather than blocking them.

The non-overridable invariants are the **enforcement mechanics** of
the filter proxy itself: no raw-socket bypass, fail-closed binds for
unadvertised globals, and minijail process isolation.  Those cannot be
changed through the option surface.

## Secure default: zero warnings

A configuration using only nixling's built-in feature bundles with no
custom rules, no `appIdPrefix` override, and no `titlePrefix` override
produces zero `waylandFilter` warnings.  If you see unexpected
warnings with a stock configuration, please report them.
