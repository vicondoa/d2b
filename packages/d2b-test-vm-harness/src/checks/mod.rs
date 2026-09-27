//! The checks whose assertions are the lane's own Rust.
//!
//! The lane's checks started as `runNixOSTest` fixtures: a nix file declared
//! the guest and a Python `testScript` made the assertions, and the lane ran
//! both. A check is ported when its assertions move here, into a function
//! over the same guest-control surface an unported check's script is given
//! ([`GuestControl`]), and its fixture is retired in the same change. Until
//! its own port a check keeps running its evaluated `check.py` through
//! [`crate::legacy`], so the lane carries any mix of the two.
//!
//! Two things a port leaves alone. The guest is not the check's to move: the
//! node it boots is declared where the reusable nodes are
//! (`nix/test-support/host-integration-node.nix`), and the image action still
//! builds that guest from that declaration. And the diagnostics are not the
//! check's to invent: [`GuestControl::stage`], [`GuestControl::diag_unit`]
//! and [`GuestControl::diag_wait`] are the primitives the fixtures'
//! diagnostics prelude provides, so a ported check that fails prints the
//! stage it was in, the rows it was asserting on, and the journal and zone
//! dump that explain them, exactly as the fixture's failure did.
//!
//! [`GuestControl::stage`]: crate::legacy::GuestControl::stage
//! [`GuestControl::diag_unit`]: crate::legacy::GuestControl::diag_unit
//! [`GuestControl::diag_wait`]: crate::legacy::GuestControl::diag_wait
//! [`GuestControl`]: crate::legacy::GuestControl

pub mod bridge_isolation;
pub mod daemon_smoke;
pub mod guest_agent_cap_confinement;
pub mod guest_shell_service;
pub mod privilege_oracle;
pub mod state_posture_contract;
pub mod resource_operator_activation;
pub mod wayland_proxy;

use crate::legacy::{GuestControl, LegacyResult};

/// One check's assertions: the guest-control surface, asserted against in the
/// order the check's own fixture asserted them.
pub type Assertions = fn(&mut GuestControl) -> LegacyResult<()>;

/// The checks that assert in Rust, by the name the lane reports them under.
///
/// One entry per ported check, and the entry is that check's own module. The
/// image says which side of this list a check is on (its manifest carries
/// whether it holds an evaluated script), so a check whose image says it is
/// ported and which has no entry here is a lane failure rather than a check
/// that quietly does not run.
const PORTED: &[(&str, Assertions)] = &[
    ("bridge-isolation", bridge_isolation::assertions),
    ("daemon-smoke", daemon_smoke::assertions),
    (
        "guest-agent-cap-confinement",
        guest_agent_cap_confinement::assertions,
    ),
    ("guest-shell-service", guest_shell_service::assertions),
    ("privilege-oracle", privilege_oracle::assertions),
    (
        "resource-operator-activation",
        resource_operator_activation::assertions,
    ),
    (
        "state-posture-contract",
        state_posture_contract::assertions,
    ),
    ("wayland-proxy", wayland_proxy::assertions),
];

/// The assertions of one ported check, or `None` for a check that has not
/// been ported.
pub fn assertions(name: &str) -> Option<Assertions> {
    PORTED
        .iter()
        .find(|(ported, _)| *ported == name)
        .map(|(_, assertions)| *assertions)
}
