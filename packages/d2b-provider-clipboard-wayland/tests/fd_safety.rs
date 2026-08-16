use d2b_provider_clipboard_wayland::{
    FdObjectKind, FdStatModel, FileSystemKind, classify_fd_model,
};

#[test]
fn disk_backed_regular_files_are_rejected() {
    assert!(
        classify_fd_model(FdStatModel {
            object_kind: FdObjectKind::Regular,
            filesystem_kind: FileSystemKind::DiskBacked,
        })
        .is_err()
    );
}
