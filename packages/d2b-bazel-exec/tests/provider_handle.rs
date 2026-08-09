use std::{ffi::OsStr, io, path::Path};

use d2b_bazel_exec::classify_exec_error;
use d2b_bazel_support::fsops::{
    Digest, FileKind, FileSystem, InMemoryFileSystem, OpenRoute, ResolveMask, ResolvePolicy,
    Timestamp, verify_provider,
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
fn provider_verification_consumes_one_descriptor_without_exposing_it() {
    let (filesystem, root, digest) = fixture(OpenRoute::OpenAt2);
    let provider = filesystem
        .open_provider(&root, Path::new(PROVIDER))
        .expect("provider open");
    verify_provider(&filesystem, provider, None, digest).expect("provider should verify");
    assert_eq!(filesystem.provider_open_count(), 1);
}

#[test]
fn provider_open_uses_only_no_magiclinks_and_keeps_strict_paths_strict() {
    let (filesystem, root, digest) = fixture(OpenRoute::OpenAt2);
    let provider = filesystem
        .open_provider(&root, Path::new(PROVIDER))
        .expect("provider open");
    verify_provider(&filesystem, provider, None, digest).expect("provider should verify");

    let provider_record = filesystem
        .open_records()
        .iter()
        .find(|record| record.policy == ResolvePolicy::Provider)
        .copied()
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
fn forced_component_walk_keeps_intermediate_components_strict() {
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
    let verified = verify_provider(
        &filesystem,
        provider,
        None,
        Digest::sha256(b"outside-provider"),
    );
    assert!(verified.is_ok());
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
fn provider_verification_rejects_bad_kind_mode_freshness_bytes_and_races() {
    let filesystem = InMemoryFileSystem::new();
    let root = filesystem.root();
    filesystem
        .add_directory(&root, OsStr::new(PROVIDER))
        .expect("directory");
    let provider = filesystem
        .open_provider(&root, Path::new(PROVIDER))
        .expect("directory provider open");
    assert!(matches!(
        verify_provider(&filesystem, provider, None, [0; 32]),
        Err(d2b_bazel_support::fsops::VerificationError::NotRegular)
    ));

    let (filesystem, root, digest) = fixture(OpenRoute::OpenAt2);
    let provider = filesystem
        .open_provider(&root, Path::new(PROVIDER))
        .expect("provider open");
    filesystem.set_provider_mode(&provider, 0o644);
    assert!(matches!(
        verify_provider(&filesystem, provider, None, digest),
        Err(d2b_bazel_support::fsops::VerificationError::NotExecutable)
    ));

    let (filesystem, root, digest) = fixture(OpenRoute::OpenAt2);
    filesystem.set_short_read(Some(1));
    let provider = filesystem
        .open_provider(&root, Path::new(PROVIDER))
        .expect("provider open");
    assert!(matches!(
        verify_provider(&filesystem, provider, None, digest),
        Err(d2b_bazel_support::fsops::VerificationError::ShortRead)
    ));

    let (filesystem, root, digest) = fixture(OpenRoute::OpenAt2);
    filesystem.set_metadata_change_after_pread(true);
    let provider = filesystem
        .open_provider(&root, Path::new(PROVIDER))
        .expect("provider open");
    assert!(matches!(
        verify_provider(&filesystem, provider, None, digest),
        Err(d2b_bazel_support::fsops::VerificationError::MetadataChanged)
    ));
}

#[test]
fn provider_verification_rejects_stale_inputs_and_digest_mismatches() {
    let filesystem = InMemoryFileSystem::new();
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
        .expect("provider");
    filesystem
        .add_file(
            &root,
            OsStr::new("newest-input"),
            b"input",
            0o644,
            Timestamp::new(11, 0),
            Timestamp::new(11, 0),
        )
        .expect("newest input");

    let provider = filesystem
        .open_provider(&root, Path::new(PROVIDER))
        .expect("provider open");
    let newest_input = filesystem
        .open_provider(&root, Path::new("newest-input"))
        .expect("input open");
    assert!(matches!(
        verify_provider(
            &filesystem,
            provider,
            Some(&newest_input),
            Digest::sha256(b"verified-provider"),
        ),
        Err(d2b_bazel_support::fsops::VerificationError::Stale)
    ));

    let provider = filesystem
        .open_provider(&root, Path::new(PROVIDER))
        .expect("provider reopen");
    assert!(matches!(
        verify_provider(&filesystem, provider, None, Digest::sha256(b"other")),
        Err(d2b_bazel_support::fsops::VerificationError::DigestMismatch)
    ));
}

#[test]
fn execveat_enosys_is_a_named_refusal_without_path_fallback() {
    let error = io::Error::from_raw_os_error(rustix::io::Errno::NOSYS.raw_os_error());
    assert_eq!(
        classify_exec_error(&error),
        d2b_bazel_exec::ExecErrno::Enosys
    );
}

#[test]
fn support_handles_have_fixed_debug_renderings() {
    let (filesystem, root, _) = fixture(OpenRoute::OpenAt2);
    let provider = filesystem
        .open_provider(&root, Path::new(PROVIDER))
        .expect("provider open");
    assert_eq!(format!("{provider:?}"), "ProviderHandle(..)");
    assert_eq!(format!("{root:?}"), "FsHandle(..)");
    assert_eq!(
        format!("{:?}", filesystem.fstat(&root).expect("metadata")),
        "FileMetadata(..)"
    );
    assert_eq!(format!("{:?}", FileKind::Regular), "Regular");
}
