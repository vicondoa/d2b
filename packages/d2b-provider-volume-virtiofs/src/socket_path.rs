//! The private listening socket path one binding's serving worker binds.
//!
//! The path is DERIVED from the admitted binding, never parsed out of a
//! launch argument and never read out of a Guest's device rows. It is a
//! pure function of two inputs this crate owns: the broker-owned runtime
//! root the daemon composes into the launch, and the binding's own opaque
//! [`SocketIdentity`].
//!
//! That is what makes a Guest with no Device children deliver storage
//! (AE6). A per-Guest runtime directory shared with the Device workers is
//! postured from a `path:vm-run:<guest>` row that a Device-free Guest
//! never causes to exist, so deriving the serving socket from it made the
//! export preparation refuse exactly the Guests that need it least. The
//! socket therefore lands directly in the runtime root the broker already
//! owns and has already postured: no per-Guest directory is created, no
//! directory mode is invented, and no create race with a sibling worker's
//! grant exists.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::bindings::SocketIdentity;

/// Linux's maximum Unix-domain socket path length.
pub const MAX_SOCKET_PATH_BYTES: usize = 108;

/// How many bytes of the socket identity reach the socket's file name.
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
/// The result is anchored, normalized, strictly a direct child of
/// `runtime_root`, and within [`MAX_SOCKET_PATH_BYTES`]. It is a pure
/// function of its two arguments, so re-deriving it after a helper
/// restart yields byte-identical output: a restarted worker rebinds the
/// same socket rather than a second one, and two different bindings never
/// collide on one name.
pub fn derive_serving_socket_path(
    runtime_root: &Path,
    socket: &SocketIdentity,
) -> Result<PathBuf, SocketPathRefusal> {
    if !is_anchored_runtime_root(runtime_root) {
        return Err(SocketPathRefusal::RuntimeRootInvalid);
    }
    let tag = &socket.to_hex()[..SOCKET_TAG_BYTES * 2];
    let rendered = format!("{}/vfd-{tag}.sock", runtime_root.display());
    if rendered.len() > MAX_SOCKET_PATH_BYTES {
        return Err(SocketPathRefusal::PathTooLong);
    }
    Ok(PathBuf::from(rendered))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use d2b_contracts_resource::v3::ResourceRef;
    use d2b_contracts_resource::v3::execution_policy::BoundedToken;

    use super::{MAX_SOCKET_PATH_BYTES, SocketPathRefusal, derive_serving_socket_path};
    use crate::bindings::SocketIdentity;

    fn identity(tail: u8) -> SocketIdentity {
        SocketIdentity::derive(
            &BoundedToken::parse("dev").expect("token"),
            &ResourceRef::parse("Volume/work-state").expect("valid ref"),
            &ResourceRef::parse(format!("Guest/work-vm-{tail}").as_str()).expect("valid ref"),
            &BoundedToken::parse("ro-store").expect("valid view"),
        )
    }

    /// The derivation is total over an anchored root, so the export
    /// preparation of a Guest that has no device rows is not gated on any
    /// other resource existing.
    #[test]
    fn the_socket_is_a_direct_child_of_the_runtime_root() {
        let path = derive_serving_socket_path(Path::new("/run/d2b"), &identity(0))
            .expect("an anchored root derives a socket");
        assert_eq!(path.parent(), Some(Path::new("/run/d2b")));
        assert!(path.is_absolute());
        assert!(path.as_os_str().as_encoded_bytes().len() <= MAX_SOCKET_PATH_BYTES);
    }

    /// Re-derivation after a helper restart is byte-identical: the restart
    /// rebinds the same socket instead of exposing a second one.
    #[test]
    fn the_derivation_is_stable_and_distinct_per_binding() {
        let first = derive_serving_socket_path(Path::new("/run/d2b"), &identity(0))
            .expect("socket");
        let restarted =
            derive_serving_socket_path(Path::new("/run/d2b"), &identity(0))
                .expect("socket");
        let other = derive_serving_socket_path(Path::new("/run/d2b"), &identity(1))
            .expect("socket");
        assert_eq!(first, restarted);
        assert_ne!(first, other);
    }

    /// Every root that cannot be fenced with a component comparison is
    /// refused rather than normalized, and a root that cannot fit a socket
    /// address is refused rather than truncated.
    #[test]
    fn an_unusable_runtime_root_or_an_over_long_name_is_refused() {
        let socket = identity(0);
        for root in [
            "/run/d2b/",
            "run/d2b",
            "/run/../etc",
            "/run//d2b",
            "/run/d2b/./x",
            "/run/d2\\b",
        ] {
            assert_eq!(
                derive_serving_socket_path(Path::new(root), &socket),
                Err(SocketPathRefusal::RuntimeRootInvalid),
                "an unanchored root is refused: {root}"
            );
        }
        let deep = format!("/run/{}", "d".repeat(96));
        assert_eq!(
            derive_serving_socket_path(Path::new(&deep), &socket),
            Err(SocketPathRefusal::PathTooLong)
        );
    }
}
