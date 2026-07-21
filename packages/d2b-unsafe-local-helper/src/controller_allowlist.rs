//! Narrow, exact-UID authorization for the `runtime-systemd-user` endpoint.
//!
//! The frozen `EndpointPolicy` for this endpoint declares
//! `EndpointRole::LocalRootController` as its initiator, but the helper
//! always executes as the exact authenticated requesting uid and provides
//! no isolation boundary of its own (see
//! `docs/explanation/unsafe-local-runtime.md`). The only production peers
//! that can legitimately reach this per-user socket from a different uid are
//! the fixed, Nix-derived controller identities of the enabled host-local
//! realms that declare this requester in their `allowedUsers`.
//!
//! This module resolves that bounded, sorted, exact set of controller UIDs
//! from an immutable Nix-owned document. It never selects, changes, or
//! influences which uid the helper *executes as* — it only answers "is this
//! already-authenticated peer uid allowed to open a session", which stays a
//! pure boolean decision at accept time.

use std::collections::BTreeSet;

/// Bound on the number of realm controller UIDs a single requester can be
/// paired with. Real deployments have a small number of enabled host-local
/// realms; anything past this is treated as malformed/untrusted input and
/// fails the whole document closed.
pub const MAX_CONTROLLER_UIDS_PER_USER: usize = 64;

/// Bound on the number of requester rows the document can carry. Keeps
/// parsing cost fixed and turns a runaway or corrupted file into a
/// fail-closed rejection instead of unbounded work.
pub const MAX_ALLOWLIST_ENTRIES: usize = 256;

const SUPPORTED_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControllerAllowlistError {
    Malformed,
    UnsupportedSchema,
    TooManyEntries,
    TooManyControllerUids,
    DuplicateUser,
    DuplicateControllerUid,
    UnsortedControllerUids,
    ZeroControllerUid,
}

#[derive(serde::Deserialize)]
struct AllowlistDocument {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    entries: Vec<AllowlistEntry>,
}

#[derive(serde::Deserialize)]
struct AllowlistEntry {
    user: String,
    #[serde(rename = "controllerUids")]
    controller_uids: Vec<u32>,
}

/// The exact, bounded set of realm-controller UIDs authorized to reach this
/// requester's endpoint. Never constructed except through [`Self::resolve`]
/// or [`Self::empty`], so an admission decision can never be built from
/// anything other than a validated, deduplicated, sorted, Nix-owned
/// document, or from no additional grants at all.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ControllerAllowlist(BTreeSet<u32>);

impl ControllerAllowlist {
    /// No additional cross-uid grants. This is also the safe default when
    /// the Nix-owned document has not been wired in at all.
    pub fn empty() -> Self {
        Self(BTreeSet::new())
    }

    /// Whether `uid` is an exact, authorized controller. Uid `0` is never
    /// authorized, defensively, even if it could somehow appear in the set.
    pub fn contains(&self, uid: u32) -> bool {
        uid != 0 && self.0.contains(&uid)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Parses the immutable Nix-owned allowlist document and returns only
    /// the exact, bounded set of controller UIDs authorized for
    /// `requester_user`. Any malformed, duplicate, unsorted, unbounded, or
    /// zero-uid content fails the *whole* document closed rather than
    /// permissively skipping the offending row, so a corrupt or tampered
    /// file can never be interpreted as "no extra grants" for one user while
    /// silently keeping a bad row for another.
    pub fn resolve(
        document: &[u8],
        requester_user: &str,
    ) -> Result<Self, ControllerAllowlistError> {
        let document: AllowlistDocument =
            serde_json::from_slice(document).map_err(|_| ControllerAllowlistError::Malformed)?;
        if document.schema_version != SUPPORTED_SCHEMA_VERSION {
            return Err(ControllerAllowlistError::UnsupportedSchema);
        }
        if document.entries.len() > MAX_ALLOWLIST_ENTRIES {
            return Err(ControllerAllowlistError::TooManyEntries);
        }

        let mut seen_users = BTreeSet::new();
        let mut matched: Option<BTreeSet<u32>> = None;
        for entry in &document.entries {
            if entry.user.is_empty() {
                return Err(ControllerAllowlistError::Malformed);
            }
            if !seen_users.insert(entry.user.as_str()) {
                return Err(ControllerAllowlistError::DuplicateUser);
            }
            if entry.controller_uids.len() > MAX_CONTROLLER_UIDS_PER_USER {
                return Err(ControllerAllowlistError::TooManyControllerUids);
            }

            let mut uids = BTreeSet::new();
            let mut previous: Option<u32> = None;
            for &controller_uid in &entry.controller_uids {
                if controller_uid == 0 {
                    return Err(ControllerAllowlistError::ZeroControllerUid);
                }
                if let Some(previous_uid) = previous {
                    if controller_uid == previous_uid {
                        return Err(ControllerAllowlistError::DuplicateControllerUid);
                    }
                    if controller_uid < previous_uid {
                        return Err(ControllerAllowlistError::UnsortedControllerUids);
                    }
                }
                previous = Some(controller_uid);
                uids.insert(controller_uid);
            }

            if entry.user == requester_user {
                matched = Some(uids);
            }
        }
        Ok(Self(matched.unwrap_or_default()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(entries: &[(&str, &[u32])]) -> Vec<u8> {
        let entries: Vec<serde_json::Value> = entries
            .iter()
            .map(|(user, uids)| {
                serde_json::json!({
                    "user": user,
                    "controllerUids": uids,
                })
            })
            .collect();
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "entries": entries,
        }))
        .unwrap()
    }

    #[test]
    fn resolves_exact_controller_uids_for_the_matching_user() {
        let bytes = document(&[("alice", &[1234, 1300]), ("bob", &[1400])]);
        let allowlist = ControllerAllowlist::resolve(&bytes, "alice").unwrap();
        assert!(allowlist.contains(1234));
        assert!(allowlist.contains(1300));
        assert!(!allowlist.contains(1400));
        assert_eq!(allowlist.len(), 2);
    }

    #[test]
    fn unrelated_user_has_no_entry_and_resolves_empty() {
        let bytes = document(&[("bob", &[1400])]);
        let allowlist = ControllerAllowlist::resolve(&bytes, "alice").unwrap();
        assert!(allowlist.is_empty());
        assert!(!allowlist.contains(1400));
    }

    #[test]
    fn root_controller_uid_is_never_authorized_even_if_present() {
        // A malformed document could never legitimately carry uid 0 (see
        // `zero_controller_uid_fails_the_whole_document_closed`), but
        // `contains` still refuses it defensively so a bug in `resolve`
        // could never turn into a same-as-root admission.
        let allowlist = ControllerAllowlist(BTreeSet::from([0, 1234]));
        assert!(!allowlist.contains(0));
        assert!(allowlist.contains(1234));
    }

    #[test]
    fn empty_allowlist_authorizes_nothing() {
        assert!(ControllerAllowlist::empty().is_empty());
        assert!(!ControllerAllowlist::empty().contains(1234));
    }

    #[test]
    fn malformed_json_fails_closed() {
        assert_eq!(
            ControllerAllowlist::resolve(b"not-json", "alice"),
            Err(ControllerAllowlistError::Malformed)
        );
    }

    #[test]
    fn unsupported_schema_version_fails_closed() {
        let bytes = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 2,
            "entries": [],
        }))
        .unwrap();
        assert_eq!(
            ControllerAllowlist::resolve(&bytes, "alice"),
            Err(ControllerAllowlistError::UnsupportedSchema)
        );
    }

    #[test]
    fn duplicate_user_rows_fail_the_whole_document_closed() {
        let bytes = document(&[("alice", &[1234]), ("alice", &[1300])]);
        assert_eq!(
            ControllerAllowlist::resolve(&bytes, "alice"),
            Err(ControllerAllowlistError::DuplicateUser)
        );
    }

    #[test]
    fn duplicate_controller_uid_fails_the_whole_document_closed() {
        let bytes = document(&[("alice", &[1234, 1234])]);
        assert_eq!(
            ControllerAllowlist::resolve(&bytes, "alice"),
            Err(ControllerAllowlistError::DuplicateControllerUid)
        );
    }

    #[test]
    fn unsorted_controller_uids_fail_the_whole_document_closed() {
        let bytes = document(&[("alice", &[1300, 1234])]);
        assert_eq!(
            ControllerAllowlist::resolve(&bytes, "alice"),
            Err(ControllerAllowlistError::UnsortedControllerUids)
        );
    }

    #[test]
    fn zero_controller_uid_fails_the_whole_document_closed() {
        let bytes = document(&[("alice", &[0])]);
        assert_eq!(
            ControllerAllowlist::resolve(&bytes, "alice"),
            Err(ControllerAllowlistError::ZeroControllerUid)
        );
    }

    #[test]
    fn too_many_entries_fail_closed() {
        let entries: Vec<(String, Vec<u32>)> = (0..=MAX_ALLOWLIST_ENTRIES)
            .map(|index| (format!("user-{index}"), vec![]))
            .collect();
        let borrowed: Vec<(&str, &[u32])> = entries
            .iter()
            .map(|(user, uids)| (user.as_str(), uids.as_slice()))
            .collect();
        let bytes = document(&borrowed);
        assert_eq!(
            ControllerAllowlist::resolve(&bytes, "alice"),
            Err(ControllerAllowlistError::TooManyEntries)
        );
    }

    #[test]
    fn too_many_controller_uids_per_user_fail_closed() {
        let uids: Vec<u32> = (1..=(MAX_CONTROLLER_UIDS_PER_USER as u32 + 1)).collect();
        let bytes = document(&[("alice", &uids)]);
        assert_eq!(
            ControllerAllowlist::resolve(&bytes, "alice"),
            Err(ControllerAllowlistError::TooManyControllerUids)
        );
    }
}
