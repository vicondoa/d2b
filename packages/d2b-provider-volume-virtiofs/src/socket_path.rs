//! The private listening socket path one binding's serving worker binds.
//!
//! The path is DERIVED from the admitted binding, never parsed out of a
//! launch argument and never read out of a Guest's device rows. It is a
//! pure function of four inputs every side already holds: the broker-owned
//! runtime root the daemon composes into the launch, and the relationship's
//! Zone, source Volume, and consumer Guest.
//!
//! The rendering is the frozen worker socket contract (ADR 046):
//! `<runtime_root>/vms/<guest>/vol-<tag>.vfd.sock`, where the tag is the
//! first eight hex digits of `sha256(zone 0 volume 0 guest)`. The socket
//! lives inside the per-Guest runtime tree the broker already owns and
//! already declares in the verified storage contract as
//! `path:vm-run:<guest>`, alongside the Device workers' sockets, which is
//! what lets the broker grant the serving principal a named ACL entry on
//! one tree instead of on the whole runtime root.
//!
//! That placement is a fence, not a preference: the broker's serving-worker
//! grant opens the socket's parent directory to the worker principal, and
//! it refuses a parent that IS the runtime root rather than grant `rwx` on
//! a tree that also holds the broker's own sockets. A socket derived
//! directly into the runtime root therefore cannot be launched at all, and
//! one derived outside it is refused for reaching above a tree the broker
//! does not own.

use std::fmt;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use d2b_contracts_resource::v3::execution_policy::BoundedToken;

/// Linux's maximum Unix-domain socket path length.
pub const MAX_SOCKET_PATH_BYTES: usize = 108;

/// How many bytes of the relationship digest reach the socket's file name.
///
/// The name is an opaque tag, not an identity: the full digest stays
/// private and the tag only has to be collision-free among the bindings
/// one runtime root serves.
const SOCKET_TAG_BYTES: usize = 8;

/// The stable failure code each socket-path refusal renders.
const CODE_ROOT_INVALID: &str = "serving-socket-root-invalid";
const CODE_PATH_TOO_LONG: &str = "serving-socket-path-too-long";

/// Why a private serving socket path could not be derived.
///
/// The set is closed and neither code echoes the runtime root, the socket
/// identity, or any other host path: a refusal names the condition, not
/// the material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SocketPathRefusal {
    /// The runtime root is not an absolute, normalized, NUL-free path.
    ///
    /// A relative root, a trailing separator, an empty or `.`/`..`
    /// component, or an embedded NUL is refused rather than normalized,
    /// because every consumer of this path fences it with a
    /// `starts_with` comparison against the broker's own runtime root and
    /// `/run/d2b/../etc` is a component prefix of `/run/d2b` while
    /// resolving outside it.
    RuntimeRootInvalid,
    /// The derived path does not fit in a Unix-domain socket address.
    ///
    /// Linux caps `sun_path` at 108 bytes, so a long runtime root is a
    /// refusal rather than a silently truncated path.
    PathTooLong,
}

impl SocketPathRefusal {
    /// The stable `^[a-z][a-z0-9-]*$` code for this refusal.
    pub const fn code(self) -> &'static str {
        match self {
            Self::RuntimeRootInvalid => CODE_ROOT_INVALID,
            Self::PathTooLong => CODE_PATH_TOO_LONG,
        }
    }
}

impl fmt::Display for SocketPathRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for SocketPathRefusal {}

/// Whether `root` is an absolute, normalized, NUL-free directory prefix.
///
/// Shared by the derivation and by the fence the broker's own grant
/// applies, so the two cannot disagree about what an anchored root is.
pub fn is_anchored_runtime_root(root: &Path) -> bool {
    root.to_str()
        .is_some_and(|root| {
            root.starts_with('/')
                && !root.ends_with('/')
                && !root.contains('\0')
                && !root.contains('\\')
                && !root
                    .split('/')
                    .skip(1)
                    .any(|component| component.is_empty() || component == "." || component == "..")
        })
}

/// Derive the private listening socket path for one binding.
///
/// The result is anchored, normalized, strictly inside `runtime_root`
/// (the per-Guest runtime tree is a child of it), and within
/// [`MAX_SOCKET_PATH_BYTES`]. It is a pure function of its arguments, so
/// re-deriving it after a helper restart yields byte-identical output: a
/// restarted worker rebinds the same socket rather than a second one, and
/// two different bindings never collide on one name.
///
/// The three relationship tokens are [`BoundedToken`]s, so a name that
/// could escape the tree is rejected before it reaches the rendering
/// rather than normalized after it.
pub fn derive_serving_socket_path(
    runtime_root: &Path,
    zone: &BoundedToken,
    volume: &BoundedToken,
    guest: &BoundedToken,
) -> Result<PathBuf, SocketPathRefusal> {
    if !is_anchored_runtime_root(runtime_root) {
        return Err(SocketPathRefusal::RuntimeRootInvalid);
    }
    let tag = relationship_tag(zone, volume, guest);
    let rendered = format!(
        "{}/vms/{}/vol-{tag}.vfd.sock",
        runtime_root.display(),
        guest.as_str(),
    );
    if rendered.len() > MAX_SOCKET_PATH_BYTES {
        return Err(SocketPathRefusal::PathTooLong);
    }
    Ok(PathBuf::from(rendered))
}

/// The eight hex digits of `sha256(zone 0 volume 0 guest)` that name one
/// relationship's socket inside a runtime tree.
fn relationship_tag(zone: &BoundedToken, volume: &BoundedToken, guest: &BoundedToken) -> String {
    let mut hasher = Sha256::new();
    hasher.update(zone.as_str().as_bytes());
    hasher.update([0u8]);
    hasher.update(volume.as_str().as_bytes());
    hasher.update([0u8]);
    hasher.update(guest.as_str().as_bytes());
    let digest = hasher.finalize();
    let mut tag = String::with_capacity(SOCKET_TAG_BYTES * 2);
    for byte in digest.iter().take(SOCKET_TAG_BYTES / 2) {
        tag.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
        tag.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
    }
    tag
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use d2b_contracts_resource::v3::execution_policy::BoundedToken;

    use super::{MAX_SOCKET_PATH_BYTES, SocketPathRefusal, derive_serving_socket_path};

    fn token(value: &str) -> BoundedToken {
        BoundedToken::parse(value.to_owned()).expect("bounded token")
    }

    fn path(guest: &str) -> std::path::PathBuf {
        derive_serving_socket_path(
            Path::new("/run/d2b"),
            &token("work"),
            &token("state"),
            &token(guest),
        )
        .expect("an anchored root derives a socket")
    }

    /// The socket lands in the per-Guest runtime tree the broker declares
    /// as `path:vm-run:<guest>`, which is strictly inside the broker's own
    /// runtime root - the shape the broker's serving-worker ACL grant
    /// admits. A socket whose parent IS the runtime root is refused there,
    /// so this is the placement, not a preference.
    #[test]
    fn the_socket_is_inside_the_per_guest_runtime_tree() {
        let derived = path("acceptance-guest");
        assert_eq!(
            derived.parent(),
            Some(Path::new("/run/d2b/vms/acceptance-guest"))
        );
        assert!(derived.starts_with(Path::new("/run/d2b")));
        assert!(
            derived.as_os_str().as_encoded_bytes().len() <= MAX_SOCKET_PATH_BYTES,
            "the derived address fits a Unix socket"
        );
    }

    /// The rendered tag is the frozen worker contract's
    /// `sha256(zone 0 volume 0 guest)` prefix, which is what the daemon's
    /// socket probe and the acceptance fixture both assert.
    #[test]
    fn the_name_carries_the_frozen_relationship_tag() {
        assert_eq!(
            path("acceptance-guest").file_name().and_then(|n| n.to_str()),
            Some("vol-a424ba7a.vfd.sock")
        );
    }

    /// Re-derivation after a helper restart is byte-identical: the restart
    /// rebinds the same socket instead of exposing a second one.
    #[test]
    fn the_derivation_is_stable_and_distinct_per_binding() {
        let first = path("acceptance-guest");
        let restarted = path("acceptance-guest");
        let other = path("work-vm");
        assert_eq!(first, restarted);
        assert_ne!(first, other);
    }

    /// Every root that cannot be fenced with a component comparison is
    /// refused rather than normalized, and a root that cannot fit a socket
    /// address is refused rather than truncated.
    #[test]
    fn an_unusable_runtime_root_or_an_over_long_name_is_refused() {
        for root in [
            "/run/d2b/",
            "run/d2b",
            "/run/../etc",
            "/run//d2b",
            "/run/d2b/./x",
            "/run/d2\\b",
        ] {
            assert_eq!(
                derive_serving_socket_path(
                    Path::new(root),
                    &token("work"),
                    &token("state"),
                    &token("acceptance-guest"),
                ),
                Err(SocketPathRefusal::RuntimeRootInvalid),
                "an unanchored root is refused: {root}"
            );
        }
        let deep = format!("/run/{}", "d".repeat(96));
        assert_eq!(
            derive_serving_socket_path(
                Path::new(&deep),
                &token("work"),
                &token("state"),
                &token("acceptance-guest"),
            ),
            Err(SocketPathRefusal::PathTooLong)
        );
    }
}
