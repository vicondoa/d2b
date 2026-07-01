//! Contract tests for ADR 0042 clipboard architecture invariants.
//!
//! These tests run from `tests/tools/rust-workspace-checks.sh` against the
//! real checkout (the crate is excluded from the hermetic Nix sandbox build).
//! They use `read_repo_file` / `repo_root` helpers to read real repo files.
//!
//! Invariants covered:
//!   1. `flake.nix` exports `d2b-clipd` as a named package attribute.
//!   2. The `d2b` CLI source declares a `Clipboard` subcommand.
//!   3. `d2b-wayland-proxy` never hides `wl_data_device_manager` via the
//!      deny/hidden-globals path; it must use the synthetic virtualizer.

use d2b_contract_tests::read_repo_file;

// ---------------------------------------------------------------------------
// 1. flake.nix exports d2b-clipd
// ---------------------------------------------------------------------------

/// The `flake.nix` must expose `d2b-clipd` as a named package so NixOS
/// consumers and the CI `nix build .#d2b-clipd` invocation can resolve it.
///
/// Regression gate: if `d2b-clipd` is accidentally removed from the flake
/// packages attrset (e.g. stripped during a merge), this test fails before
/// any Nix build gate is needed.
#[test]
fn d2b_clipd_exported_in_flake_nix() {
    let flake = read_repo_file("flake.nix");

    // The package must appear as an attribute assignment of the form
    // `d2b-clipd = ...` or `"d2b-clipd" = ...` in the packages attrset.
    assert!(
        flake.contains("d2b-clipd = rustWorkspace")
            || flake.contains("d2b-clipd = d2bLib")
            || flake.contains(r#""d2b-clipd""#)
            || (flake.contains("d2b-clipd") && flake.contains("pname = \"d2b-clipd\"")),
        "flake.nix must export d2b-clipd as a named package attribute. \
         ADR 0042 requires d2b-clipd to be independently buildable via \
         `nix build .#d2b-clipd`."
    );
}

// ---------------------------------------------------------------------------
// 2. d2b CLI declares Clipboard subcommand
// ---------------------------------------------------------------------------

/// The `d2b` CLI must expose a `Clipboard` top-level subcommand for the
/// ADR 0042 explicit picker-driven paste action (`d2b clipboard arm`).
///
/// Regression gate: if the `Clipboard` variant is removed from `NativeCommand`
/// (breaking the `d2b clipboard arm` operator path), this test fails.
#[test]
fn d2b_cli_has_clipboard_subcommand() {
    let lib = read_repo_file("packages/d2b/src/lib.rs");

    // The NativeCommand enum must have a Clipboard variant.
    assert!(
        lib.contains("Clipboard(ClipboardArgs)") || lib.contains("Clipboard(clipboard_args"),
        "packages/d2b/src/lib.rs must declare a `Clipboard(ClipboardArgs)` variant in \
         the NativeCommand enum. ADR 0042 requires `d2b clipboard arm` as the \
         explicit picker-driven paste command."
    );

    // The dispatch block must handle the Clipboard arm.
    assert!(
        lib.contains("NativeCommand::Clipboard"),
        "packages/d2b/src/lib.rs must handle NativeCommand::Clipboard in the dispatch block."
    );

    // The command must have at least an Arm subverb.
    assert!(
        lib.contains("ClipboardCommand::Arm") || lib.contains("clipboard arm"),
        "packages/d2b/src/lib.rs must declare a ClipboardCommand::Arm subverb \
         for the explicit picker-driven paste workflow (ADR 0042 §fallback)."
    );
}

// ---------------------------------------------------------------------------
// 3. Wayland filter must NOT hide wl_data_device_manager via deny path
// ---------------------------------------------------------------------------

/// `d2b-wayland-proxy/src/filter.rs` must advertise `wl_data_device_manager`
/// to guests using the synthetic virtualizer path (`synthetic_clipboard: true`)
/// and must NEVER add it to `hidden_globals` (the deny path).
///
/// ADR 0042 requires the filter to intercept and synthesize guest clipboard
/// objects rather than hiding the global. Hiding the global breaks guest apps
/// that use standard clipboard (copy/paste stops working entirely).
///
/// The synthetic_clipboard path advertises the global to the guest but
/// intercepts all bind calls, routing them through the in-process virtualizer
/// instead of forwarding to the host compositor's real global.
#[test]
fn wayland_filter_advertises_not_hides_wl_data_device_manager() {
    let filter = read_repo_file("packages/d2b-wayland-proxy/src/filter.rs");

    // The filter must use synthetic_clipboard: true for wl_data_device_manager.
    assert!(
        filter.contains("synthetic_clipboard: true"),
        "packages/d2b-wayland-proxy/src/filter.rs must contain \
         `synthetic_clipboard: true` for the wl_data_device_manager bind path. \
         ADR 0042 requires the filter to advertise (not hide) this global so \
         guest apps can use standard clipboard via the synthesizer."
    );

    // The wl_data_device_manager early-return must come BEFORE the
    // hidden_globals.insert deny path in the handle_global function.
    // This structural assertion prevents a future refactor from accidentally
    // routing wl_data_device_manager through the deny path.
    let data_device_pos = filter
        .find("wl_data_device_manager")
        .expect("filter.rs must reference wl_data_device_manager");
    let hidden_insert_pos = filter
        .find("hidden_globals.insert")
        .expect("filter.rs must have hidden_globals.insert for the deny path");
    assert!(
        data_device_pos < hidden_insert_pos,
        "The wl_data_device_manager special-case in filter.rs (pos {data_device_pos}) \
         must appear BEFORE the hidden_globals.insert deny path (pos {hidden_insert_pos}). \
         If this assertion fails, wl_data_device_manager may have been accidentally \
         moved into the deny path, breaking guest clipboard."
    );

    // The filter.rs must NOT contain a direct deny for wl_data_device_manager.
    // This check catches any pattern like:
    //   if iface == "wl_data_device_manager" { hidden_globals.insert(...) }
    // by verifying the two strings never appear on the same source line.
    for line in filter.lines() {
        assert!(
            !(line.contains("wl_data_device_manager") && line.contains("hidden_globals")),
            "filter.rs line must not combine wl_data_device_manager with hidden_globals: {line:?}. \
             ADR 0042 forbids hiding this global."
        );
    }
}

#[test]
fn clipboard_policy_no_longer_declares_a_substrate_gap() {
    let clipboard = read_repo_file("packages/d2b-wayland-proxy/src/clipboard.rs");

    assert!(
        !clipboard.contains("WL_PROXY_CLIPBOARD_SUBSTRATE_GAP"),
        "packages/d2b-wayland-proxy/src/clipboard.rs must not reintroduce the \
         substrate-gap marker now that the proxy has a synthetic clipboard path."
    );
}
