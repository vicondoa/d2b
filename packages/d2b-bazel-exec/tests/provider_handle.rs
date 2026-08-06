use std::{ffi::OsStr, io, path::Path};

use d2b_bazel_exec::{ProviderError, classify_exec_error, provider::verify_provider};
use d2b_bazel_support::{
    fsops::{
        Digest, FileKind, FileSystem, InMemoryFileSystem, OpenRoute, ResolveMask, ResolvePolicy,
        Timestamp,
    },
    runfiles::{InMemoryRunfilesView, RunfilesLookup, RunfilesMode, RunfilesView},
};

const PROVIDER: &str = "provider";

fn fixture(
    route: OpenRoute,
) -> (
    InMemoryFileSystem,
    d2b_bazel_support::fsops::FsHandle,
    Digest,
) {
    let filesystem = InMemoryFileSystem::with_route(route);
    let root = filesystem.root();
    filesystem
        .add_file(
            &root,
            OsStr::new(PROVIDER),
            b"verified-provider",
            0o755,
            Timestamp::new(10, 0),
            Timestamp::new(10, 0),
        )
        .expect("provider fixture");
    (filesystem, root, Digest::sha256(b"verified-provider"))
}

#[test]
fn complete_provider_verification_consumes_one_descriptor_and_checks_identity() {
    let (filesystem, root, digest) = fixture(OpenRoute::OpenAt2);
    let provider = filesystem
        .open_provider(&root, Path::new(PROVIDER))
        .expect("provider open");

    let verified =
        verify_provider(&filesystem, provider, None, digest).expect("provider should verify");
    assert_eq!(filesystem.provider_open_count(), 1);
    drop(verified);
}

#[test]
fn provider_open_uses_only_no_magiclinks_and_keeps_strict_paths_strict() {
    let (filesystem, root, digest) = fixture(OpenRoute::OpenAt2);
    let provider = filesystem
        .open_provider(&root, Path::new(PROVIDER))
        .expect("provider open");
    verify_provider(&filesystem, provider, None, digest).expect("provider should verify");

    let records = filesystem.open_records();
    let provider_record = records
        .iter()
        .find(|record| record.policy == ResolvePolicy::Provider)
        .expect("provider record");
    assert_eq!(provider_record.resolve_flags, ResolveMask::NO_MAGICLINKS);
    assert!(!provider_record.resolve_flags.contains(ResolveMask::BENEATH));
    assert!(
        !provider_record
            .resolve_flags
            .contains(ResolveMask::NO_SYMLINKS)
    );

    filesystem
        .openat2(
            &root,
            Path::new(PROVIDER),
            d2b_bazel_support::fsops::OpenFlags::RDONLY,
            ResolvePolicy::Strict,
        )
        .expect("strict fixture open");
    let strict = filesystem
        .open_records()
        .into_iter()
        .find(|record| record.policy == ResolvePolicy::Strict)
        .expect("strict record");
    assert!(strict.resolve_flags.contains(ResolveMask::NO_MAGICLINKS));
    assert!(strict.resolve_flags.contains(ResolveMask::NO_SYMLINKS));
    assert!(strict.resolve_flags.contains(ResolveMask::BENEATH));
}

#[test]
fn forced_component_walk_uses_intermediate_nofollow_and_permissive_leaf() {
    let filesystem = InMemoryFileSystem::with_route(OpenRoute::ComponentWalk);
    let root = filesystem.root();
    let nested = filesystem
        .add_directory(&root, OsStr::new("nested"))
        .expect("nested directory");
    let target = filesystem
        .add_file(
            &root,
            OsStr::new("outside"),
            b"outside-provider",
            0o755,
            Timestamp::new(10, 0),
            Timestamp::new(10, 0),
        )
        .expect("outside provider");
    filesystem
        .add_symlink(&nested, OsStr::new(PROVIDER), &target)
        .expect("provider leaf symlink");

    let provider = filesystem
        .open_provider(&root, Path::new("nested/provider"))
        .expect("provider leaf symlink is allowed");
    assert_eq!(provider.inode(), target.inode());
    let record = filesystem
        .open_records()
        .into_iter()
        .find(|record| record.policy == ResolvePolicy::Provider)
        .expect("provider walk record");
    assert!(record.intermediate_no_follow);
    assert_eq!(record.resolve_flags, ResolveMask::NO_MAGICLINKS);

    let strict = filesystem
        .open_component_walk(
            &nested,
            Path::new(PROVIDER),
            d2b_bazel_support::fsops::OpenFlags::RDONLY,
            ResolvePolicy::Strict,
        )
        .expect_err("strict leaf must reject the symlink");
    assert_eq!(
        strict.raw_os_error(),
        Some(rustix::io::Errno::LOOP.raw_os_error())
    );
}

#[test]
fn runfiles_mode_is_selected_once_and_missing_bazel_entries_do_not_fallback() {
    let (filesystem, root, digest) = fixture(OpenRoute::OpenAt2);
    let present = InMemoryRunfilesView::present("/runfiles/root", [Path::new(PROVIDER).into()]);
    assert_eq!(present.mode(), RunfilesMode::Bazel);
    assert!(matches!(
        present.lookup(Path::new(PROVIDER)),
        RunfilesLookup::Present(_)
    ));
    assert!(matches!(
        InMemoryRunfilesView::cargo().lookup(Path::new(PROVIDER)),
        RunfilesLookup::NotBazel
    ));
    let _ = (filesystem, root, digest);
}

#[test]
fn escaping_runfiles_leaf_is_verified_by_descriptor_not_anchor_prefix() {
    let filesystem = InMemoryFileSystem::new();
    let root = filesystem.root();
    let outside = filesystem
        .add_file(
            &root,
            OsStr::new("outside"),
            b"outside-provider",
            0o755,
            Timestamp::new(10, 0),
            Timestamp::new(10, 0),
        )
        .expect("outside file");
    let runfiles = filesystem
        .add_directory(&root, OsStr::new("runfiles"))
        .expect("runfiles directory");
    filesystem
        .add_symlink(&runfiles, OsStr::new(PROVIDER), &outside)
        .expect("escaping leaf");
    let provider = filesystem
        .open_provider(&runfiles, Path::new(PROVIDER))
        .expect("permissive provider leaf");
    assert_eq!(provider.inode(), outside.inode());
    let bytes = b"outside-provider";
    let verified = d2b_bazel_support::fsops::verify_provider(
        &filesystem,
        provider,
        None,
        Digest::sha256(bytes),
    )
    .expect("escaped leaf is still digest checked");
    let (provider, _, _) = verified.into_parts();
    assert_eq!(provider.inode(), outside.inode());
}

#[test]
fn enosys_falls_back_to_component_walk_without_changing_provider_policy() {
    let (filesystem, root, digest) = fixture(OpenRoute::OpenAt2);
    filesystem.set_openat2_error(Some(rustix::io::Errno::NOSYS.raw_os_error()));
    let provider = filesystem
        .open_provider(&root, Path::new(PROVIDER))
        .expect("component walk fallback");
    verify_provider(&filesystem, provider, None, digest).expect("fallback provider verifies");
    let records = filesystem.open_records();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].route, OpenRoute::OpenAt2);
    assert_eq!(records[1].route, OpenRoute::ComponentWalk);
}

#[test]
fn provider_descriptors_are_read_only_and_close_on_exec() {
    let (filesystem, root, digest) = fixture(OpenRoute::OpenAt2);
    let provider = filesystem
        .open_provider(&root, Path::new(PROVIDER))
        .expect("provider open");
    assert!(provider.is_close_on_exec().expect("fake fd flags"));
    assert_eq!(
        filesystem.open_records()[0].flags,
        d2b_bazel_support::fsops::OpenFlags::RDONLY | d2b_bazel_support::fsops::OpenFlags::CLOEXEC
    );
    verify_provider(&filesystem, provider, None, digest).expect("provider verifies");
}

#[test]
fn provider_verification_rejects_bad_kind_mode_freshness_bytes_and_metadata_race() {
    let filesystem = InMemoryFileSystem::new();
    let root = filesystem.root();
    let directory = filesystem
        .add_directory(&root, OsStr::new(PROVIDER))
        .expect("directory");
    let error = verify_provider(
        &filesystem,
        filesystem
            .open_provider(&root, Path::new(PROVIDER))
            .expect("directory provider open"),
        None,
        [0; 32],
    )
    .err()
    .expect("directory is not executable provider");
    assert!(matches!(error, ProviderError::Verification(_)));
    let _ = directory;

    let (filesystem, root, digest) = fixture(OpenRoute::OpenAt2);
    let provider = filesystem
        .open_provider(&root, Path::new(PROVIDER))
        .expect("provider open");
    filesystem.set_mode(provider.descriptor(), 0o644);
    let error = verify_provider(&filesystem, provider, None, digest)
        .err()
        .expect("mode must fail");
    assert!(matches!(
        error,
        ProviderError::Verification(
            d2b_bazel_support::fsops::VerificationError::NotExecutable { .. }
        )
    ));

    let (filesystem, root, digest) = fixture(OpenRoute::OpenAt2);
    let input = filesystem
        .add_file(
            &root,
            OsStr::new("newer"),
            b"input",
            0o644,
            Timestamp::new(11, 0),
            Timestamp::new(11, 0),
        )
        .expect("newer input");
    let provider = filesystem
        .open_provider(&root, Path::new(PROVIDER))
        .expect("provider open");
    let newest = filesystem
        .open_provider(&root, Path::new("newer"))
        .expect("input open");
    let error = verify_provider(&filesystem, provider, Some(&newest), digest)
        .err()
        .expect("stale provider must fail");
    assert!(matches!(
        error,
        ProviderError::Verification(d2b_bazel_support::fsops::VerificationError::Stale { .. })
    ));
    let _ = input;

    let (filesystem, root, digest) = fixture(OpenRoute::OpenAt2);
    filesystem.set_short_read(Some(1));
    let provider = filesystem
        .open_provider(&root, Path::new(PROVIDER))
        .expect("provider open");
    let error = verify_provider(&filesystem, provider, None, digest)
        .err()
        .expect("short digest read must fail");
    assert!(matches!(
        error,
        ProviderError::Verification(d2b_bazel_support::fsops::VerificationError::ShortRead { .. })
    ));

    let (filesystem, root, digest) = fixture(OpenRoute::OpenAt2);
    filesystem.set_metadata_change_after_pread(true);
    let provider = filesystem
        .open_provider(&root, Path::new(PROVIDER))
        .expect("provider open");
    let error = verify_provider(&filesystem, provider, None, digest)
        .err()
        .expect("metadata race must fail");
    assert!(matches!(
        error,
        ProviderError::Verification(d2b_bazel_support::fsops::VerificationError::MetadataChanged)
    ));
}

#[test]
fn execveat_enosys_is_a_named_refusal_without_path_fallback() {
    let error = io::Error::from_raw_os_error(rustix::io::Errno::NOSYS.raw_os_error());
    assert_eq!(
        classify_exec_error(&error),
        d2b_bazel_exec::ExecErrno::Enosys
    );
    assert_ne!(
        classify_exec_error(&error),
        d2b_bazel_exec::ExecErrno::Other
    );
}

#[test]
fn provider_handle_does_not_change_kind_when_the_path_is_rebound() {
    let (filesystem, root, digest) = fixture(OpenRoute::OpenAt2);
    let provider = filesystem
        .open_provider(&root, Path::new(PROVIDER))
        .expect("provider open");
    let replacement = filesystem
        .add_file(
            &root,
            OsStr::new("replacement"),
            b"replacement",
            0o755,
            Timestamp::new(10, 0),
            Timestamp::new(10, 0),
        )
        .expect("replacement");
    filesystem
        .rebind(&root, OsStr::new(PROVIDER), &replacement)
        .expect("rebind path");
    let verified = d2b_bazel_support::fsops::verify_provider(&filesystem, provider, None, digest);
    assert!(
        verified.is_ok(),
        "the already-open descriptor remains authoritative"
    );
    assert_eq!(filesystem.inode_named(PROVIDER), replacement.inode());
}

#[test]
fn file_kind_and_digest_helpers_remain_typed() {
    let (filesystem, root, digest) = fixture(OpenRoute::OpenAt2);
    let descriptor = filesystem
        .open_provider(&root, Path::new(PROVIDER))
        .expect("provider open");
    let metadata = filesystem.fstat(descriptor.descriptor()).expect("metadata");
    assert_eq!(metadata.kind(), FileKind::Regular);
    assert!(metadata.is_executable());
    assert_eq!(digest, Digest::sha256(b"verified-provider"));
}
