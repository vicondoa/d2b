//! Bridge-port defaults and fail-closed readback validation.

/// The semantic role of one Network-owned bridge port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TapRole {
    /// The net VM's LAN-facing port.
    NetVmLan,
    /// A workload port with direct east-west traffic disabled.
    WorkloadLanIsolated,
    /// A workload port with explicitly enabled east-west traffic.
    WorkloadLanEastWest,
    /// The point-to-point net VM uplink.
    UplinkPointToPoint,
}

/// Every bridge-port flag written and read back by the effect adapter.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct BridgePortFlagSet {
    /// Block forwarding directly between isolated bridge ports.
    pub isolated: bool,
    /// Reflect frames back to their incoming port.
    pub hairpin_mode: bool,
    /// Learn source addresses on this port.
    pub learning: bool,
    /// Flood unknown unicast frames to this port.
    pub unicast_flood: bool,
    /// Flood multicast frames to this port.
    pub multicast_flood: bool,
    /// Suppress neighbour discovery on this port.
    pub neigh_suppress: bool,
    /// Reject bridge protocol data units from this port.
    pub bpdu_guard: bool,
    /// Prevent this port from becoming a spanning-tree root port.
    pub root_block: bool,
    /// Leave multicast groups immediately when membership ends.
    pub fast_leave: bool,
}

impl core::fmt::Debug for BridgePortFlagSet {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("BridgePortFlagSet(<redacted>)")
    }
}

impl BridgePortFlagSet {
    /// A neutral initial state before role policy is applied.
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

    /// Return the complete established flag set for a semantic port role.
    pub const fn defaults_for(role: TapRole) -> Self {
        match role {
            TapRole::NetVmLan => Self {
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
            TapRole::WorkloadLanIsolated => Self {
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
            TapRole::WorkloadLanEastWest => Self {
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
            TapRole::UplinkPointToPoint => Self {
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

/// One closed bridge-port flag difference.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct FlagDifference {
    /// The closed flag name.
    pub flag: &'static str,
    /// The policy value.
    pub expected: bool,
    /// The observed value.
    pub actual: bool,
}

impl core::fmt::Debug for FlagDifference {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("FlagDifference(<redacted>)")
    }
}

/// A complete, identity-free bridge-port drift report.
#[derive(Clone, PartialEq, Eq)]
pub struct BridgePortFlagDrift {
    /// The semantic port role.
    pub role: TapRole,
    /// Every differing flag.
    pub differences: Vec<FlagDifference>,
}

impl core::fmt::Debug for BridgePortFlagDrift {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("BridgePortFlagDrift(<redacted>)")
    }
}

impl core::fmt::Display for BridgePortFlagDrift {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("bridge-port-flag-drift")
    }
}

impl std::error::Error for BridgePortFlagDrift {}

/// Compare every observed flag with the complete role policy.
pub fn validate_readback(
    role: TapRole,
    observed: BridgePortFlagSet,
) -> Result<(), BridgePortFlagDrift> {
    let expected = BridgePortFlagSet::defaults_for(role);
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

/// The two explicit authoring decisions required for direct east-west traffic.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct EastWestPolicy {
    /// The Network explicitly enables east-west traffic.
    pub network_allows_east_west: bool,
    /// Host policy explicitly allows the unsafe direct-L2 mode.
    pub host_allows_unsafe_east_west: bool,
}

impl core::fmt::Debug for EastWestPolicy {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("EastWestPolicy(<redacted>)")
    }
}

/// Closed rejection reasons for bridge-port role admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgePortPolicyError {
    /// Network policy did not enable direct east-west traffic.
    NetworkOptInRequired,
    /// Host policy did not enable unsafe direct-L2 traffic.
    HostOptInRequired,
}

impl core::fmt::Display for BridgePortPolicyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::NetworkOptInRequired => "east-west-network-opt-in-required",
            Self::HostOptInRequired => "east-west-host-opt-in-required",
        })
    }
}

impl std::error::Error for BridgePortPolicyError {}

/// Refuse the direct east-west role unless both policy layers opt in.
pub fn validate_role_against_policy(
    role: TapRole,
    policy: EastWestPolicy,
) -> Result<(), BridgePortPolicyError> {
    if role != TapRole::WorkloadLanEastWest {
        return Ok(());
    }
    if !policy.network_allows_east_west {
        return Err(BridgePortPolicyError::NetworkOptInRequired);
    }
    if !policy.host_allows_unsafe_east_west {
        return Err(BridgePortPolicyError::HostOptInRequired);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readback_matches_defaults() {
        for role in [
            TapRole::NetVmLan,
            TapRole::WorkloadLanIsolated,
            TapRole::WorkloadLanEastWest,
            TapRole::UplinkPointToPoint,
        ] {
            validate_readback(role, BridgePortFlagSet::defaults_for(role)).unwrap();
        }
    }

    #[test]
    fn drift_fails_closed_and_lists_every_difference() {
        let mut observed = BridgePortFlagSet::defaults_for(TapRole::WorkloadLanIsolated);
        observed.isolated = false;
        observed.neigh_suppress = false;
        let drift = validate_readback(TapRole::WorkloadLanIsolated, observed).unwrap_err();
        assert_eq!(drift.differences.len(), 2);
        assert_eq!(drift.to_string(), "bridge-port-flag-drift");
    }

    #[test]
    fn direct_east_west_requires_both_opt_ins() {
        let role = TapRole::WorkloadLanEastWest;
        assert_eq!(
            validate_role_against_policy(
                role,
                EastWestPolicy {
                    network_allows_east_west: false,
                    host_allows_unsafe_east_west: true,
                },
            ),
            Err(BridgePortPolicyError::NetworkOptInRequired)
        );
        assert_eq!(
            validate_role_against_policy(
                role,
                EastWestPolicy {
                    network_allows_east_west: true,
                    host_allows_unsafe_east_west: false,
                },
            ),
            Err(BridgePortPolicyError::HostOptInRequired)
        );
    }
}
