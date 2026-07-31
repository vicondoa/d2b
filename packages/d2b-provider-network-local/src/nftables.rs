//! Ownership-scoped `inet d2b` firewall projection logic.
//!
//! This module deliberately has no whole-table replacement operation. Apply and
//! remove mutate one trusted Network slot, preserve every sibling and foreign
//! entry byte-for-byte, and reject a marker mismatch in the target slot.

use d2b_contracts::v3::ResourceUid;

/// The mandatory ownership marker prefix for every managed chain and rule.
pub const OWNERSHIP_MARKER_PREFIX: &str = "d2b managed: ";

/// The four Network-owned chains in canonical order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NetworkChainHook {
    /// Pre-routing filtering.
    Prerouting,
    /// Forwarding policy.
    Forward,
    /// Host output policy.
    Output,
    /// Host input policy.
    Input,
}

impl NetworkChainHook {
    fn name(self) -> &'static str {
        match self {
            Self::Prerouting => "prerouting",
            Self::Forward => "forward",
            Self::Output => "output",
            Self::Input => "input",
        }
    }

    fn priority(self) -> i32 {
        match self {
            Self::Prerouting => -150,
            Self::Forward | Self::Output | Self::Input => -5,
        }
    }

    fn policy(self) -> &'static str {
        match self {
            Self::Forward => "drop",
            Self::Prerouting | Self::Output | Self::Input => "accept",
        }
    }
}

/// A validated Network-owned nftables rule expression.
pub struct NetworkRule(String);

impl NetworkRule {
    // Only crate-owned semantic renderers may create expressions. A public raw
    // expression constructor would make USBIP ownership impossible to seal.
    #[allow(dead_code)]
    fn parse(expression: impl Into<String>) -> Result<Self, NftablesError> {
        let expression = expression.into();
        let normalized = expression.to_ascii_lowercase();
        if expression.trim().is_empty()
            || expression.contains(['\n', '\r'])
            || normalized.contains("comment")
            || normalized.contains("delete table")
            || normalized.contains("flush table")
            || normalized.contains("add table")
            || normalized.contains("usbip")
            || normalized
                .split(|character: char| !character.is_ascii_alphanumeric())
                .any(|token| token == "3240")
        {
            return Err(NftablesError::InvalidRule);
        }
        Ok(Self(expression))
    }

    fn render(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for NetworkRule {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("NetworkRule(<redacted>)")
    }
}

/// One canonical Network-owned chain projection.
#[derive(Debug)]
pub struct NetworkChain {
    hook: NetworkChainHook,
    rules: Vec<NetworkRule>,
}

impl NetworkChain {
    /// Construct a chain for one of the four closed hooks.
    pub fn new(hook: NetworkChainHook, rules: Vec<NetworkRule>) -> Self {
        Self { hook, rules }
    }
}

/// A complete Network-owned projection for one ownership identity.
pub struct NetworkNftProjection {
    owner: ResourceUid,
    chains: Vec<NetworkChain>,
}

impl NetworkNftProjection {
    /// Construct a projection containing each canonical chain exactly once.
    pub fn new(owner: ResourceUid, mut chains: Vec<NetworkChain>) -> Result<Self, NftablesError> {
        chains.sort_by_key(|chain| chain.hook);
        let hooks: Vec<_> = chains.iter().map(|chain| chain.hook).collect();
        if hooks
            != [
                NetworkChainHook::Prerouting,
                NetworkChainHook::Forward,
                NetworkChainHook::Output,
                NetworkChainHook::Input,
            ]
        {
            return Err(NftablesError::InvalidChainLayout);
        }
        Ok(Self { owner, chains })
    }

    /// Build the four-chain layout with no rules.
    pub fn empty(owner: ResourceUid) -> Self {
        Self {
            owner,
            chains: vec![
                NetworkChain::new(NetworkChainHook::Prerouting, Vec::new()),
                NetworkChain::new(NetworkChainHook::Forward, Vec::new()),
                NetworkChain::new(NetworkChainHook::Output, Vec::new()),
                NetworkChain::new(NetworkChainHook::Input, Vec::new()),
            ],
        }
    }

    fn render(&self) -> Vec<u8> {
        let marker = format!("{OWNERSHIP_MARKER_PREFIX}{}", self.owner.as_str());
        let mut rendered = String::new();
        for chain in &self.chains {
            rendered.push_str(&format!(
                "chain {} {{ type filter hook {} priority {}; policy {}; comment \"{}\";\n",
                chain.hook.name(),
                chain.hook.name(),
                chain.hook.priority(),
                chain.hook.policy(),
                marker,
            ));
            for rule in &chain.rules {
                rendered.push_str(&format!("  {} comment \"{}\"\n", rule.render(), marker,));
            }
            rendered.push_str("}\n");
        }
        rendered.into_bytes()
    }
}

impl core::fmt::Debug for NetworkNftProjection {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("NetworkNftProjection(<redacted>)")
    }
}

#[derive(Clone)]
enum EntrySlot {
    Network(ResourceUid),
    Foreign,
}

#[derive(Clone)]
enum ObservedMarker {
    Managed(ResourceUid),
    Foreign,
}

/// One observed byte range inside the shared table.
#[derive(Clone)]
pub struct SharedTableEntry {
    slot: EntrySlot,
    marker: ObservedMarker,
    bytes: Vec<u8>,
}

impl SharedTableEntry {
    /// Validate and record a correctly marked Network projection.
    pub fn managed(owner: ResourceUid, bytes: Vec<u8>) -> Result<Self, NftablesError> {
        if !has_exact_managed_markers(&owner, &bytes) {
            return Err(NftablesError::ForeignMarkerPreserved);
        }
        Ok(Self::managed_rendered(owner, bytes))
    }

    fn managed_rendered(owner: ResourceUid, bytes: Vec<u8>) -> Self {
        Self {
            slot: EntrySlot::Network(owner.clone()),
            marker: ObservedMarker::Managed(owner),
            bytes,
        }
    }

    /// Record an unrelated foreign table entry that must be preserved.
    pub fn foreign(bytes: Vec<u8>) -> Self {
        Self {
            slot: EntrySlot::Foreign,
            marker: ObservedMarker::Foreign,
            bytes,
        }
    }

    /// Record a foreign marker occupying a trusted Network slot.
    pub fn foreign_in_network_slot(owner: ResourceUid, bytes: Vec<u8>) -> Self {
        Self {
            slot: EntrySlot::Network(owner),
            marker: ObservedMarker::Foreign,
            bytes,
        }
    }

    /// Borrow exact bytes for the core effect adapter.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl core::fmt::Debug for SharedTableEntry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SharedTableEntry(<redacted>)")
    }
}

/// An observed shared `inet d2b` table split into ownership entries.
#[derive(Clone)]
pub struct SharedNftTable {
    entries: Vec<SharedTableEntry>,
}

impl SharedNftTable {
    /// Construct a snapshot without interpreting or rewriting unrelated bytes.
    pub fn new(entries: Vec<SharedTableEntry>) -> Self {
        Self { entries }
    }

    /// Borrow entries in their exact table order.
    pub fn entries(&self) -> &[SharedTableEntry] {
        &self.entries
    }
}

impl core::fmt::Debug for SharedNftTable {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SharedNftTable(<redacted>)")
    }
}

/// Opaque SHA-256 digest of one Network ownership projection.
#[derive(Clone, PartialEq, Eq)]
pub struct FirewallDigest([u8; 32]);

impl FirewallDigest {
    /// Render hexadecimal bytes for the bounded provider status field.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

impl core::fmt::Debug for FirewallDigest {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("FirewallDigest(<redacted>)")
    }
}

/// Result of one projection-scoped table update.
pub struct ProjectionUpdate {
    table: SharedNftTable,
    digest: Option<FirewallDigest>,
}

impl ProjectionUpdate {
    /// Borrow the updated shared table.
    pub const fn table(&self) -> &SharedNftTable {
        &self.table
    }

    /// Borrow the applied projection digest, or `None` after removal.
    pub const fn digest(&self) -> Option<&FirewallDigest> {
        self.digest.as_ref()
    }
}

impl core::fmt::Debug for ProjectionUpdate {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ProjectionUpdate(<redacted>)")
    }
}

/// Closed, value-free nftables rejection reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NftablesError {
    /// Rule text is empty, unsafe, or belongs to another Provider.
    InvalidRule,
    /// The projection does not contain exactly the four canonical chains.
    InvalidChainLayout,
    /// A foreign marker occupies the Network's trusted ownership slot.
    ForeignMarkerPreserved,
    /// More than one entry claims the same trusted ownership slot.
    AmbiguousOwnership,
    /// The declared firewall coexistence mode does not match observation.
    FirewallCoexistenceMismatch,
}

impl NftablesError {
    /// Return the stable, redacted reason code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidRule => "network-nft-rule-invalid",
            Self::InvalidChainLayout => "network-nft-chain-layout-invalid",
            Self::ForeignMarkerPreserved => "foreign-nft-rule-preserved",
            Self::AmbiguousOwnership => "network-nft-ownership-ambiguous",
            Self::FirewallCoexistenceMismatch => "firewall-coexistence-mismatch",
        }
    }
}

impl core::fmt::Display for NftablesError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for NftablesError {}

/// Apply exactly one Network projection while preserving all other entries.
pub fn apply_projection(
    snapshot: &SharedNftTable,
    projection: &NetworkNftProjection,
) -> Result<ProjectionUpdate, NftablesError> {
    let rendered = projection.render();
    let digest = digest(&rendered);
    let mut entries = snapshot.entries.clone();
    let matching: Vec<_> = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| matches!(&entry.slot, EntrySlot::Network(owner) if owner == &projection.owner))
        .map(|(index, _)| index)
        .collect();
    if matching.len() > 1 {
        return Err(NftablesError::AmbiguousOwnership);
    }
    if let Some(index) = matching.first().copied() {
        match &entries[index].marker {
            ObservedMarker::Managed(owner) if owner == &projection.owner => {
                entries[index] =
                    SharedTableEntry::managed_rendered(projection.owner.clone(), rendered);
            }
            ObservedMarker::Managed(_) | ObservedMarker::Foreign => {
                return Err(NftablesError::ForeignMarkerPreserved);
            }
        }
    } else {
        entries.push(SharedTableEntry::managed_rendered(
            projection.owner.clone(),
            rendered,
        ));
    }
    Ok(ProjectionUpdate {
        table: SharedNftTable::new(entries),
        digest: Some(digest),
    })
}

/// Remove exactly one validated Network projection.
///
/// A validated absent slot is idempotent success. A conflicting marker is
/// preserved and rejected rather than removed.
pub fn remove_projection(
    snapshot: &SharedNftTable,
    owner: &ResourceUid,
) -> Result<ProjectionUpdate, NftablesError> {
    let matching: Vec<_> = snapshot
        .entries
        .iter()
        .enumerate()
        .filter(
            |(_, entry)| matches!(&entry.slot, EntrySlot::Network(candidate) if candidate == owner),
        )
        .map(|(index, _)| index)
        .collect();
    if matching.len() > 1 {
        return Err(NftablesError::AmbiguousOwnership);
    }
    let mut entries = snapshot.entries.clone();
    if let Some(index) = matching.first().copied() {
        match &entries[index].marker {
            ObservedMarker::Managed(marker_owner) if marker_owner == owner => {
                entries.remove(index);
            }
            ObservedMarker::Managed(_) | ObservedMarker::Foreign => {
                return Err(NftablesError::ForeignMarkerPreserved);
            }
        }
    }
    Ok(ProjectionUpdate {
        table: SharedNftTable::new(entries),
        digest: None,
    })
}

/// Hash only the selected Network ownership projection.
///
/// Sibling Network, device-owned, and foreign entry churn does not enter this
/// digest. A conflicting or ambiguous target slot fails closed.
pub fn read_projection_digest(
    snapshot: &SharedNftTable,
    owner: &ResourceUid,
) -> Result<Option<FirewallDigest>, NftablesError> {
    let matching: Vec<_> = snapshot
        .entries
        .iter()
        .filter(|entry| matches!(&entry.slot, EntrySlot::Network(candidate) if candidate == owner))
        .collect();
    match matching.as_slice() {
        [] => Ok(None),
        [entry] => match &entry.marker {
            ObservedMarker::Managed(marker_owner) if marker_owner == owner => {
                Ok(Some(digest(entry.bytes())))
            }
            ObservedMarker::Managed(_) | ObservedMarker::Foreign => {
                Err(NftablesError::ForeignMarkerPreserved)
            }
        },
        _ => Err(NftablesError::AmbiguousOwnership),
    }
}

fn digest(bytes: &[u8]) -> FirewallDigest {
    FirewallDigest(sha256(bytes))
}

fn has_exact_managed_markers(owner: &ResourceUid, bytes: &[u8]) -> bool {
    let Ok(text) = core::str::from_utf8(bytes) else {
        return false;
    };
    let expected = format!("{OWNERSHIP_MARKER_PREFIX}{}", owner.as_str());
    let mut marker_count = 0;
    let mut chains = [false; 4];
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line == "}" {
            continue;
        }
        if line.matches(" comment \"").count() != 1 {
            return false;
        }
        let Some((_, quoted)) = line.rsplit_once(" comment \"") else {
            return false;
        };
        let Some((marker, tail)) = quoted.split_once('"') else {
            return false;
        };
        if marker != expected || !matches!(tail, "" | ";") {
            return false;
        }
        if let Some(chain) = line.strip_prefix("chain ") {
            let Some(name) = chain.split_whitespace().next() else {
                return false;
            };
            let index = match name {
                "prerouting" => 0,
                "forward" => 1,
                "output" => 2,
                "input" => 3,
                _ => return false,
            };
            if chains[index] {
                return false;
            }
            chains[index] = true;
        }
        marker_count += 1;
    }
    chains.into_iter().all(core::convert::identity) && marker_count >= 4
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    const INITIAL: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];
    const ROUND: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bf_e8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];

    let bit_len = (bytes.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity((bytes.len() + 72) & !63);
    padded.extend_from_slice(bytes);
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    let mut state = INITIAL;
    for chunk in padded.chunks_exact(64) {
        let mut words = [0u32; 64];
        for (index, word) in words[..16].iter_mut().enumerate() {
            let offset = index * 4;
            *word = u32::from_be_bytes(chunk[offset..offset + 4].try_into().unwrap());
        }
        for index in 16..64 {
            let small0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let small1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(small0)
                .wrapping_add(words[index - 7])
                .wrapping_add(small1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for index in 0..64 {
            let big1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choose = (e & f) ^ (!e & g);
            let first = h
                .wrapping_add(big1)
                .wrapping_add(choose)
                .wrapping_add(ROUND[index])
                .wrapping_add(words[index]);
            let big0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let second = big0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(first);
            d = c;
            c = b;
            b = a;
            a = first.wrapping_add(second);
        }
        for (slot, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *slot = slot.wrapping_add(value);
        }
    }

    let mut output = [0u8; 32];
    for (chunk, value) in output.chunks_exact_mut(4).zip(state) {
        chunk.copy_from_slice(&value.to_be_bytes());
    }
    output
}

/// Detected host firewall manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirewallManager {
    /// firewalld owns host firewall policy.
    Firewalld,
    /// UFW owns host firewall policy.
    Ufw,
    /// Docker manages host firewall rules.
    Docker,
    /// libvirt manages host firewall rules.
    Libvirt,
    /// iptables uses the nftables backend.
    IptablesNft,
    /// Observation found an unsupported or ambiguous manager set.
    Unknown,
    /// No host firewall manager was observed.
    None,
}

/// Required coexistence behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoexistencePolicy {
    /// Refuse active coexistence with the detected manager.
    Refuse,
    /// Require d2b links to remain unmanaged by the detected manager.
    RequireUnmanaged,
    /// Permit coexistence under ownership and readback checks.
    Coexist,
}

/// Enforce the established seven-row firewall coexistence matrix.
pub fn evaluate_coexistence_policy(
    detected: FirewallManager,
    declared: CoexistencePolicy,
) -> Result<(), NftablesError> {
    let required = match detected {
        FirewallManager::Firewalld | FirewallManager::Ufw | FirewallManager::Unknown => {
            CoexistencePolicy::Refuse
        }
        FirewallManager::Docker | FirewallManager::Libvirt => CoexistencePolicy::RequireUnmanaged,
        FirewallManager::IptablesNft | FirewallManager::None => CoexistencePolicy::Coexist,
    };
    if declared == required {
        Ok(())
    } else {
        Err(NftablesError::FirewallCoexistenceMismatch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uid(value: &str) -> ResourceUid {
        ResourceUid::parse(value).unwrap()
    }

    fn managed_entry(owner: ResourceUid, rule: Option<&str>) -> SharedTableEntry {
        let projection = if let Some(rule) = rule {
            NetworkNftProjection::new(
                owner.clone(),
                vec![
                    NetworkChain::new(NetworkChainHook::Prerouting, Vec::new()),
                    NetworkChain::new(
                        NetworkChainHook::Forward,
                        vec![NetworkRule::parse(rule).unwrap()],
                    ),
                    NetworkChain::new(NetworkChainHook::Output, Vec::new()),
                    NetworkChain::new(NetworkChainHook::Input, Vec::new()),
                ],
            )
            .unwrap()
        } else {
            NetworkNftProjection::empty(owner.clone())
        };
        SharedTableEntry::managed(owner, projection.render()).unwrap()
    }

    #[test]
    fn projection_update_preserves_sibling_and_foreign_bytes() {
        let owner = uid("123e4567-e89b-42d3-a456-426614174000");
        let sibling = uid("223e4567-e89b-42d3-a456-426614174001");
        let sibling_entry = managed_entry(sibling, None);
        let sibling_bytes = sibling_entry.bytes().to_vec();
        let foreign_bytes = b"foreign table state".to_vec();
        let snapshot = SharedNftTable::new(vec![
            sibling_entry,
            SharedTableEntry::foreign(foreign_bytes.clone()),
        ]);
        let update = apply_projection(&snapshot, &NetworkNftProjection::empty(owner)).unwrap();
        assert_eq!(update.table().entries()[0].bytes(), sibling_bytes);
        assert_eq!(update.table().entries()[1].bytes(), foreign_bytes);
        assert!(update.digest().is_some());
    }

    #[test]
    fn foreign_marker_in_target_slot_fails_closed() {
        let owner = uid("123e4567-e89b-42d3-a456-426614174000");
        let bytes = b"foreign occupant".to_vec();
        let snapshot = SharedNftTable::new(vec![SharedTableEntry::foreign_in_network_slot(
            owner.clone(),
            bytes.clone(),
        )]);
        let error = apply_projection(&snapshot, &NetworkNftProjection::empty(owner)).unwrap_err();
        assert_eq!(error, NftablesError::ForeignMarkerPreserved);
        assert_eq!(snapshot.entries()[0].bytes(), bytes);
    }

    #[test]
    fn network_rules_reject_usbip_and_service_port() {
        assert!(NetworkRule::parse("tcp dport 3240 accept").is_err());
        assert!(NetworkRule::parse("meta iifname usbip-relay accept").is_err());
        assert!(NetworkRule::parse("ct state established accept").is_ok());
    }

    #[test]
    fn observed_managed_projection_validates_exact_markers() {
        let owner = uid("123e4567-e89b-42d3-a456-426614174000");
        assert_eq!(
            SharedTableEntry::managed(owner, b"foreign marker payload".to_vec()).unwrap_err(),
            NftablesError::ForeignMarkerPreserved
        );
    }

    #[test]
    fn projection_digest_ignores_sibling_and_foreign_churn() {
        let owner = uid("123e4567-e89b-42d3-a456-426614174000");
        let sibling = uid("223e4567-e89b-42d3-a456-426614174001");
        let first = SharedNftTable::new(vec![
            managed_entry(owner.clone(), None),
            managed_entry(sibling.clone(), Some("ct state established accept")),
            SharedTableEntry::foreign(b"foreign one".to_vec()),
        ]);
        let second = SharedNftTable::new(vec![
            managed_entry(owner.clone(), None),
            managed_entry(sibling, Some("ct state related accept")),
            SharedTableEntry::foreign(b"foreign two".to_vec()),
        ]);
        assert_eq!(
            read_projection_digest(&first, &owner).unwrap(),
            read_projection_digest(&second, &owner).unwrap()
        );
    }

    #[test]
    fn projection_digest_is_sha256() {
        assert_eq!(
            digest(b"").to_hex(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn coexistence_matrix_has_all_seven_rows() {
        use CoexistencePolicy::{Coexist, Refuse, RequireUnmanaged};
        use FirewallManager::{Docker, Firewalld, IptablesNft, Libvirt, None, Ufw, Unknown};
        for (manager, policy) in [
            (Firewalld, Refuse),
            (Ufw, Refuse),
            (Docker, RequireUnmanaged),
            (Libvirt, RequireUnmanaged),
            (IptablesNft, Coexist),
            (Unknown, Refuse),
            (None, Coexist),
        ] {
            evaluate_coexistence_policy(manager, policy).unwrap();
        }
        assert_eq!(
            evaluate_coexistence_policy(Firewalld, Coexist),
            Err(NftablesError::FirewallCoexistenceMismatch)
        );
    }
}
