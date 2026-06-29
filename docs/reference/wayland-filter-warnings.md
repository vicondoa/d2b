# `graphics.waylandFilter` warning catalog

> **Reference** for the advisory runtime diagnostics the d2b
> Wayland proxy emits when operator configuration deviates from
> the secure baseline or touches a rule d2b classifies as required
> or high-risk.

> **Status:** live for
> `d2b.vms.<vm>.graphics.waylandFilter.{enable,denyGlobals,allowGlobals,maxVersions,dmabufAllow,dmabufDeny,debugLogging,byteLogging}`.
> The proxy emits these diagnostics at runtime in the
> `d2b-wayland-proxy` journal stream. They are not NixOS eval-time
> `config.warnings`.

Warnings are **advisory**: the NixOS configuration still evaluates and
builds when a warning condition is met. The warning is emitted by the
host-side proxy process when the VM starts.

Secure defaults emit **zero** `waylandFilter` warnings. A clean
configuration with no overrides produces no output from this catalog.

The `W-*` names below are documentation anchors. The Rust policy engine
currently emits human-readable `PolicyWarning` messages.

## Warning conditions

### W-DENY-BASELINE

**Trigger:** `denyGlobals` explicitly denies a global that d2b
classifies as a required application-baseline global.

**Required baseline globals:** `wl_compositor`, `wl_shm`, `xdg_wm_base`,
`wl_seat`, `wl_output`, `wl_subcompositor`.

**Example:**

```nix
d2b.vms.work.graphics.waylandFilter.denyGlobals = [
  "wl_compositor"
];
```

**Why it exists:** Denying these globals breaks ordinary guest
applications. Most apps require compositor, shared-memory buffer, XDG
shell, input seat, and output objects to function at all.

**How to override intentionally:** Keep the option and acknowledge the
runtime warning in code comments. The configuration is accepted; guest
apps on this VM may not render or receive input.

---

### W-DENY-ACCEL

**Trigger:** `denyGlobals` disables a dmabuf or rendering global that
d2b expects for GPU-accelerated graphics.

**Affected globals:** `zwp_linux_dmabuf_v1`,
`wp_linux_drm_syncobj_manager_v1`, `wl_eglstream_display`, and
`wl_eglstream_controller`.

**Why it exists:** Disabling dmabuf/render globals causes guest apps to
fall back to software (llvmpipe) rendering, which significantly reduces
graphics performance and may break GPU-dependent apps.

**How to override intentionally:** Set the deny option and accept the
performance regression. The warning confirms that llvmpipe fallback is
expected.

---

### W-ALLOW-HIGH-RISK

**Trigger:** `allowGlobals` includes a global that d2b classifies as
high risk and denies by default.

**High-risk categories:**

| Category | Risk |
|---|---|
| Screen capture | Screen/image capture globals allow guest apps to capture the host display. |
| Virtual input | Virtual keyboard and pointer globals allow guest apps to inject arbitrary host input events. |
| Clipboard control | Privileged data-control globals allow guest apps to read or modify arbitrary host clipboard content. |
| Desktop shell | Layer-shell and privileged shell-surface globals give guest apps elevated compositor privileges. |
| Session control | Session lock, output power, output management, and workspace management globals give guest apps broad compositor control. |
| Security context | Wayland security-context extension is disabled until a concrete safe use case is identified. |

**Why it exists:** These protocols give guest apps abilities that exceed
ordinary window management. Enabling them extends the trust boundary from
"guest app can render windows" to "guest app can capture the screen,
inject input, or lock the session on the host".

**How to override intentionally:** Add the global to `allowGlobals` and
document the reason in the host configuration. Treat any VM with these
globals enabled as a higher-trust guest and review its isolation
(`crossDomainTrusted` justification, no privileged-container workloads).

---

### W-ALLOW-UNCLASSIFIED

**Trigger:** `allowGlobals` includes a global that d2b has not yet
classified as either a known-safe application protocol or a known-high
risk protocol.

**Why it exists:** Unclassified globals may be safe or may expose
host-side privilege. D2b defaults to denying them until classified.
This warning signals that the operator is taking responsibility for the
security posture of an unreviewed protocol.

**How to override intentionally:** Add the global to `allowGlobals` and
document in the host configuration why the global is safe for this VM.
Consider filing an issue or PR to have d2b classify the global so the
warning is resolved upstream.

## Warning vs. hard assertion

Warnings never become hard assertions. Every warning condition still
produces a valid, buildable NixOS configuration. The distinction from a
hard assertion (`lib.mkAssert` or `config.assertions`) is intentional:
operators may have valid workload-specific reasons to deviate from
d2b's baseline, and d2b should facilitate informed, documented
exceptions rather than blocking them.

The non-overridable invariants are the **enforcement mechanics** of the
Wayland proxy itself: no raw-socket bypass, fail-closed binds for
unadvertised globals, and minijail process isolation. Those cannot be
changed through the option surface.

## Secure default: zero warnings

A configuration using `waylandFilter.enable = true` with no custom
`denyGlobals` or `allowGlobals` produces zero
`waylandFilter` warnings. If you see unexpected warnings with a stock
configuration, please report them.
