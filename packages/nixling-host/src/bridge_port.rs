//! W3 host-prepare module: `bridge_port` — owned by scope s2.
//!
//! Implements the per-role bridge port flag defaults table per
//! plan.md §"W3 bridge-port flag readback (every flag, every role)"
//! plus the validators that gate east-west bridges behind the
//! `env.lan.allowEastWest` + `site.allowUnsafeEastWest` double opt-in.
//!
//! The complete flag set this module covers (every flag, every role):
//! `isolated`, `hairpin_mode`, `learning`, `unicast_flood`,
//! `multicast_flood`, `neigh_suppress`, `bpdu_guard`, `root_block`,
//! `fast_leave`.

use nixling_core::host_w3::TapRoleW3;
use serde::{Deserialize, Serialize};

/// Complete bridge-port flag readback record. Mirrors every flag the
/// W3 plan requires the broker to readback after every
/// `SetBridgePortFlags`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BridgePortFlagSet {
    pub isolated: bool,
    pub hairpin_mode: bool,
    pub learning: bool,
    pub unicast_flood: bool,
    pub multicast_flood: bool,
    pub neigh_suppress: bool,
    pub bpdu_guard: bool,
    pub root_block: bool,
    pub fast_leave: bool,
}

impl BridgePortFlagSet {
    /// All flags off; used as the starting point before applying the
    /// per-role policy.
    pub const ALL_OFF: Self = Self {
        isolated: false,
        hairpin_mode: false,
        learning: false,
        unicast_flood: false,
        multicast_flood: false,
        neigh_suppress: false,
        bpdu_guard: false,
        root_block: false,
        fast_leave: false,
    };

    /// Per-role default flag set per plan.md table.
    pub const fn defaults_for(role: TapRoleW3) -> Self {
        match role {
            // Net-VM upstream port: learning + flooding ON for L2
            // bridging; isolation OFF so the net-VM can reach every
            // workload port; neigh_suppress OFF; bpdu_guard ON so
            // foreign STP-speakers can't take over.
            TapRoleW3::NetVmLan => Self {
                isolated: false,
                hairpin_mode: false,
                learning: true,
                unicast_flood: true,
                multicast_flood: true,
                neigh_suppress: false,
                bpdu_guard: true,
                root_block: true,
                fast_leave: false,
            },
            // Workload LAN, isolated: east-west traffic blocked between
            // workload ports via the bridge isolated flag.
            TapRoleW3::WorkloadLanIsolated => Self {
                isolated: true,
                hairpin_mode: false,
                learning: true,
                unicast_flood: false,
                multicast_flood: false,
                neigh_suppress: true,
                bpdu_guard: true,
                root_block: true,
                fast_leave: true,
            },
            // Workload LAN, east-west allowed: must pass double
            // opt-in via the validator (`allowed_east_west`). Flags
            // mirror NetVmLan minus the bpdu/root guard (which only
            // makes sense on the uplink).
            TapRoleW3::WorkloadLanEastWest => Self {
                isolated: false,
                hairpin_mode: false,
                learning: true,
                unicast_flood: true,
                multicast_flood: true,
                neigh_suppress: false,
                bpdu_guard: false,
                root_block: false,
                fast_leave: false,
            },
            // Uplink point-to-point: no flooding, no learning needed
            // since there's only one peer; neigh_suppress ON.
            TapRoleW3::UplinkP2P => Self {
                isolated: true,
                hairpin_mode: false,
                learning: false,
                unicast_flood: false,
                multicast_flood: false,
                neigh_suppress: true,
                bpdu_guard: true,
                root_block: true,
                fast_leave: true,
            },
        }
    }
}

/// Result of comparing a readback flag set against the expected
/// defaults — used by the post-`SetBridgePortFlags` netlink readback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgePortFlagDrift {
    pub role: TapRoleW3,
    pub differences: Vec<FlagDifference>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct FlagDifference {
    pub flag: &'static str,
    pub expected: bool,
    pub actual: bool,
}

/// Compares an observed flag set against the per-role defaults.
/// Returns `Ok(())` if every flag matches; otherwise a drift report
/// listing every flag that diverges.
pub fn validate_readback(
    role: TapRoleW3,
    observed: BridgePortFlagSet,
) -> Result<(), BridgePortFlagDrift> {
    let expected = BridgePortFlagSet::defaults_for(role.clone());
    let mut differences = Vec::new();
    macro_rules! check {
        ($field:ident) => {
            if expected.$field != observed.$field {
                differences.push(FlagDifference {
                    flag: stringify!($field),
                    expected: expected.$field,
                    actual: observed.$field,
                });
            }
        };
    }
    check!(isolated);
    check!(hairpin_mode);
    check!(learning);
    check!(unicast_flood);
    check!(multicast_flood);
    check!(neigh_suppress);
    check!(bpdu_guard);
    check!(root_block);
    check!(fast_leave);

    if differences.is_empty() {
        Ok(())
    } else {
        Err(BridgePortFlagDrift { role, differences })
    }
}

/// Double opt-in policy for east-west bridges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EastWestPolicy {
    /// Env-level toggle: `nixling.envs.<env>.lan.allowEastWest`.
    pub env_allow_east_west: bool,
    /// Site-level toggle: `nixling.site.allowUnsafeEastWest`.
    pub site_allow_unsafe_east_west: bool,
}

/// Errors raised by [`validate_role_against_policy`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgePortPolicyError {
    /// The bundle requested `WorkloadLanEastWest` but the env did not
    /// set `lan.allowEastWest = true`.
    EastWestRequiresEnvOptIn,
    /// The bundle requested `WorkloadLanEastWest` and the env opted in
    /// but the site did not set `allowUnsafeEastWest = true`.
    EastWestRequiresSiteOptIn,
}

impl std::fmt::Display for BridgePortPolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EastWestRequiresEnvOptIn => {
                write!(f, "east-west bridge requires env.lan.allowEastWest = true")
            }
            Self::EastWestRequiresSiteOptIn => write!(
                f,
                "east-west bridge additionally requires site.allowUnsafeEastWest = true"
            ),
        }
    }
}

impl std::error::Error for BridgePortPolicyError {}

/// Refuses [`TapRoleW3::WorkloadLanEastWest`] unless **both**
/// toggles are present. Every other role is unconditionally accepted.
pub fn validate_role_against_policy(
    role: TapRoleW3,
    policy: EastWestPolicy,
) -> Result<(), BridgePortPolicyError> {
    if role != TapRoleW3::WorkloadLanEastWest {
        return Ok(());
    }
    if !policy.env_allow_east_west {
        return Err(BridgePortPolicyError::EastWestRequiresEnvOptIn);
    }
    if !policy.site_allow_unsafe_east_west {
        return Err(BridgePortPolicyError::EastWestRequiresSiteOptIn);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolated_default_blocks_east_west() {
        let f = BridgePortFlagSet::defaults_for(TapRoleW3::WorkloadLanIsolated);
        assert!(f.isolated);
        assert!(!f.unicast_flood);
    }

    #[test]
    fn east_west_default_floods() {
        let f = BridgePortFlagSet::defaults_for(TapRoleW3::WorkloadLanEastWest);
        assert!(!f.isolated);
        assert!(f.unicast_flood);
    }

    #[test]
    fn drift_lists_every_diff() {
        let mut observed = BridgePortFlagSet::defaults_for(TapRoleW3::WorkloadLanIsolated);
        observed.isolated = false;
        observed.neigh_suppress = false;
        let err = validate_readback(TapRoleW3::WorkloadLanIsolated, observed).unwrap_err();
        let flags: Vec<&'static str> = err.differences.iter().map(|d| d.flag).collect();
        assert!(flags.contains(&"isolated"));
        assert!(flags.contains(&"neigh_suppress"));
    }

    #[test]
    fn readback_matches_defaults() {
        for role in [
            TapRoleW3::NetVmLan,
            TapRoleW3::WorkloadLanIsolated,
            TapRoleW3::WorkloadLanEastWest,
            TapRoleW3::UplinkP2P,
        ] {
            validate_readback(role.clone(), BridgePortFlagSet::defaults_for(role))
                .expect("defaults pass readback");
        }
    }

    #[test]
    fn east_west_requires_env_optin() {
        let err = validate_role_against_policy(
            TapRoleW3::WorkloadLanEastWest,
            EastWestPolicy {
                env_allow_east_west: false,
                site_allow_unsafe_east_west: true,
            },
        )
        .unwrap_err();
        assert_eq!(err, BridgePortPolicyError::EastWestRequiresEnvOptIn);
    }

    #[test]
    fn east_west_requires_site_optin() {
        let err = validate_role_against_policy(
            TapRoleW3::WorkloadLanEastWest,
            EastWestPolicy {
                env_allow_east_west: true,
                site_allow_unsafe_east_west: false,
            },
        )
        .unwrap_err();
        assert_eq!(err, BridgePortPolicyError::EastWestRequiresSiteOptIn);
    }

    #[test]
    fn east_west_double_optin_accepted() {
        validate_role_against_policy(
            TapRoleW3::WorkloadLanEastWest,
            EastWestPolicy {
                env_allow_east_west: true,
                site_allow_unsafe_east_west: true,
            },
        )
        .unwrap();
    }

    #[test]
    fn non_east_west_roles_are_unconditional() {
        for role in [
            TapRoleW3::NetVmLan,
            TapRoleW3::WorkloadLanIsolated,
            TapRoleW3::UplinkP2P,
        ] {
            validate_role_against_policy(
                role,
                EastWestPolicy {
                    env_allow_east_west: false,
                    site_allow_unsafe_east_west: false,
                },
            )
            .expect("non east-west roles never require opt-in");
        }
    }
}
