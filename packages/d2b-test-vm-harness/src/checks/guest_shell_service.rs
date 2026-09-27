//! The guest ComponentSession service wiring check, ported from its fixture.
//!
//! It applies the component-session module and the guest-broker module
//! directly to a NixOS node - no nested, d2b-managed VM - and asserts that
//! the Guest target agent boots from the enrolled inputs the fixture
//! installed (a ComponentSession key pair and a self-hashed v3 bundle) and
//! reaches its AF_VSOCK ComponentSession listener, which is the route the
//! shell family's per-session supervisor service and the other provider
//! services are served over. A bundle or key the agent cannot read fails
//! closed here instead of restart-looping unnoticed.
//!
//! The assertions are the fixture's, in the fixture's order and with the
//! fixture's own command text and bounds, including the two journal greps:
//! the listener line must be there, and the bundle-validation failure line
//! must not. The diagnostics the fixture's prelude printed for those waits
//! are this surface's own: a `diag_unit` reads a unit's status, and a
//! `diag_wait` reads the row set and the journal sources the fixture named.
//!
//! The fixture declared its guest inline, on top of the two d2b modules, so
//! the guest it boots - the modules, the enrolled inputs, the bundle install
//! unit, and the `vhost-vsock-pci` device its listener needs - is declared in
//! `nix/test-support/host-integration-node.nix` beside the reusable nodes.
//!
//! `start_all()` is not restated here: it is the lane's own boot of the
//! guest the check runs against.

use std::time::Duration;

use crate::legacy::{DiagRow, GuestControl, LegacyResult};

/// The bound the guest's own `multi-user.target` gets, the fixture's own.
const BOOT: Duration = Duration::from_secs(180);

/// The bound the guest daemon gets to become active, the fixture's own.
const GUEST_DAEMON: Duration = Duration::from_secs(120);

/// The bound the listener line gets to appear in the journal, the fixture's
/// own.
const LISTENER_BOUND: Duration = Duration::from_secs(60);

/// The unit whose journal both greps read.
const GUEST_DAEMON_UNIT: &str = "d2bd-guest.service";

/// The label the row is reported under, the fixture's own: the prelude's
/// `unit_dumps` labels its dump `"<unit> status"`.
const GUEST_DAEMON_STATUS_LABEL: &str = "d2bd-guest.service status";

/// The row the listener wait explains itself with, the fixture's own: the
/// unit's status, dumped the way the prelude's `unit_dumps` dumped it.
const GUEST_DAEMON_STATUS: &str = "systemctl status d2bd-guest.service --no-pager 2>&1 | tail -n 40 || true";

/// The journal line the agent logs once its AF_VSOCK listener is bound.
const LISTENER_BOUND_COMMAND: &str =
    "journalctl -u d2bd-guest.service --no-pager -b | grep -F 'Guest ComponentSession listener bound'";

/// The bundle-validation failure the agent must not have logged.
const BUNDLE_VALIDATION_FAILED_COMMAND: &str =
    "journalctl --no-pager -b | grep -F 'Guest process bundle validation failed'";

/// The check's assertions, in the order its fixture made them.
pub fn assertions(control: &mut GuestControl) -> LegacyResult<()> {
    control.stage("boot");
    control.wait_for_unit("multi-user.target", None, BOOT)?;

    // The Guest target agent boots from the enrolled bundle and key pair,
    // and the unit that carries the device answers for it.
    control.diag_unit("guest-daemon", GUEST_DAEMON_UNIT, GUEST_DAEMON)?;
    control.succeed(&["systemctl is-active --quiet d2bd-guest.service"], None)?;

    let rows: [DiagRow<'_>; 1] = [(GUEST_DAEMON_STATUS_LABEL, GUEST_DAEMON_STATUS)];
    let explain = [("d2bd-guest.service", "")];
    control.diag_wait(
        "guest-listener-bound",
        LISTENER_BOUND_COMMAND,
        LISTENER_BOUND,
        &rows,
        &explain,
    )?;

    // ... and the failure path the same reader would have reported is
    // absent, so the listener line is evidence of a boot rather than of a
    // restart loop that got lucky.
    control.fail(&[BUNDLE_VALIDATION_FAILED_COMMAND], None)?;

    Ok(())
}
