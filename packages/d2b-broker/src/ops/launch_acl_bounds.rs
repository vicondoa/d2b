//! Broker-owned host-path bounds for one launch's per-runner ACL grants.
//!
//! Two grants the broker applies reach outside its own runtime tree: a
//! serving worker's served view root, and a runner's host session runtime
//! directory (the compositor, PipeWire and Pulse sockets). Both are selected
//! by a string that arrives on the wire - `--shared-dir=` in the launch argv,
//! `XDG_RUNTIME_DIR` / `PIPEWIRE_RUNTIME_DIR` in the plan's environment -
//! so provenance alone proves nothing about either path: any launch naming
//! any absolute host path there would have the broker open it, and open it
//! with an access ACL, to the launched principal.
//!
//! What makes such a path safe is a TRUSTED DECLARATION this module reads
//! out of the broker's own verified bundle, the same way
//! [`super::device_worker::guest_runtime_dir_posture`] reads a posture out of
//! the `path:vm-run:<guest>` row and
//! [`super::swtpm_identity::resource_backed_identity`] reads a swtpm identity
//! out of the `path:swtpm-state:<guest>` row. Both sources below are the
//! ones the daemon itself composes the ticket from
//! (`serving_worker_launch_args` for the served view root, `site.json` for
//! the session), so a well-formed launch and this derivation name the same
//! path - and a launch that names any other is refused.
//!
//! An absent declaration is `None` or empty, never a default: the caller
//! refuses the launch by name, which is the outcome the site's own
//! Wayland-less deployment and the `vm-run-dir-socket-grant` precedent both
//! already chose.

use std::path::{Path, PathBuf};

use d2b_core::bundle_resolver::BundleResolver;

/// Every shared-storage root the verified bundle declares, deduplicated and
/// sorted.
///
/// A serving worker's served view root is composed from exactly two trusted
/// sources, and this is the set of roots both compose under:
///
/// - [`BundleResolver::resolve_volume_view_root`] builds
///   `<storage row path_template>/<volume name>[/<view path>]`, so every
///   declared row's `path_template` is a root such a view root lives under;
/// - the store-view farm branch serves out of
///   [`ResolvedStoreViewIntent::hardlink_farm_path`](d2b_core::bundle_resolver::ResolvedStoreViewIntent),
///   optionally below a relative view path, so each declared farm root is a
///   root such a view root is either equal to or lives under.
///
/// A row whose template is still unexpanded (`<placeholder>`, `${...}`)
/// names no directory on this host and is dropped: it can never match, and
/// keeping it would put a root in the set that no launch could reach but
/// that still reads as broker-owned.
pub(crate) fn served_view_root_roots(resolver: &BundleResolver) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = resolver
        .storage()
        .map(|storage| {
            storage
                .paths
                .iter()
                .map(|spec| PathBuf::from(spec.path_template.as_str()))
                .collect()
        })
        .unwrap_or_default();
    roots.extend(
        resolver
            .store_view_intents()
            .map(|intent| intent.hardlink_farm_path.clone()),
    );
    roots.retain(|root| root.is_absolute() && !template_unexpanded(root));
    roots.sort();
    roots.dedup();
    roots
}

/// Whether a declared path still carries an unsubstituted template
/// placeholder, matching the storage contract's own
/// `critical-template-unexpanded` fence.
fn template_unexpanded(path: &Path) -> bool {
    path.to_string_lossy()
        .contains(['<', '>', '$', '{'])
}

/// The host session runtime directory the verified bundle declares.
///
/// `site.json` projects the site's own Wayland session socket
/// (`/run/user/<uid>/<display>`, emitted by `nixos-modules/site-json.nix`
/// from the same `d2b.site.waylandUser` / `d2b.site.waylandDisplay` options
/// the session wiring uses), so the socket's parent IS the session runtime
/// directory the audio, gpu, video, wayland-proxy and qemu-media roles
/// connect to - and the uid that owns it, which the wayland-proxy grant
/// verifies against.
///
/// `None` for a bundle that predates the artifact and for a site that
/// declares no Wayland session. Both leave the slot unbound so the launch
/// refuses by name rather than the broker naming a path no trusted artifact
/// named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRuntimeDir {
    directory: PathBuf,
    owner_uid: u32,
}

impl SessionRuntimeDir {
    /// The declared `/run/user/<uid>` directory itself.
    pub(crate) fn directory(&self) -> &Path {
        &self.directory
    }

    /// The uid the declaration says owns the directory.
    pub(crate) fn owner_uid(&self) -> u32 {
        self.owner_uid
    }

    /// Whether `candidate` is exactly the directory this declaration names.
    ///
    /// Equality, not a prefix: the ACLs this bounds reach the session's own
    /// sockets, so a directory merely below the declared one is a different
    /// directory the bundle never named.
    pub(crate) fn admits(&self, candidate: &str) -> bool {
        Path::new(candidate) == self.directory
    }
}

/// The declared session runtime directory, or `None` when the bundle
/// projects no Wayland session.
pub(crate) fn session_runtime_dir(resolver: &BundleResolver) -> Option<SessionRuntimeDir> {
    session_runtime_dir_from_site(resolver.site())
}

/// The session runtime directory one verified `site.json` declares.
pub(crate) fn session_runtime_dir_from_site(
    site: Option<&d2b_core::site::SiteJson>,
) -> Option<SessionRuntimeDir> {
    // `SiteJson` re-fences the artifact to exactly `/run/user/<uid>/<display>`,
    // so the parent projection exists; the fallible read of the owner uid
    // below keeps a malformed value unbound rather than trusted.
    let directory = site?.session_runtime_directory()?;
    Some(SessionRuntimeDir {
        directory: directory.to_path_buf(),
        owner_uid: directory.file_name()?.to_str()?.parse().ok()?,
    })
}

/// The declaration itself, for a test that drives the fences over a
/// temporary tree. Production builds only ever get this through
/// [`session_runtime_dir_from_site`], whose `site.json` shape check is what
/// makes the value trustworthy.
#[cfg(test)]
pub(crate) fn session_runtime_dir_for_test(
    directory: PathBuf,
    owner_uid: u32,
) -> SessionRuntimeDir {
    SessionRuntimeDir {
        directory,
        owner_uid,
    }
}
