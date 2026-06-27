import json
import os
import secrets
import stat
import sys

if sys.argv[1] == "-":
    specs = json.load(sys.stdin)
else:
    specs = json.load(open(sys.argv[1], encoding="utf-8"))
current_vm = "<unknown>"


def fail(kind):
    print(f"d2b guest-control token {current_vm}: {kind}", file=sys.stderr)
    sys.exit(1)


def reject_symlink_components(path, *, require_existing_file, check_parent_permissions):
    if not os.path.isabs(path):
        fail("path-not-absolute")
    current = "/"
    parts = [part for part in path.split("/") if part]
    check_parts = parts if require_existing_file else parts[:-1]
    for part in check_parts:
        current = os.path.join(current, part)
        try:
            st = os.lstat(current)
        except FileNotFoundError:
            if require_existing_file:
                fail("path-component-missing")
            break
        if stat.S_ISLNK(st.st_mode):
            fail("path-component-symlink")
        if check_parent_permissions and stat.S_ISDIR(st.st_mode) and st.st_uid != 0:
            fail("path-component-not-root-owned")
        if check_parent_permissions and stat.S_ISDIR(st.st_mode) and (
            stat.S_IMODE(st.st_mode) & 0o022
        ):
            fail("path-component-group-or-world-writable")


def validate_materialized(path, reader_gid):
    reject_symlink_components(path, require_existing_file=True, check_parent_permissions=False)
    try:
        fd = os.open(path, os.O_RDONLY | os.O_NONBLOCK | os.O_NOFOLLOW | os.O_CLOEXEC)
    except OSError:
        fail("materialized-open-failed")
    try:
        st = os.fstat(fd)
        if not stat.S_ISREG(st.st_mode):
            fail("materialized-not-regular")
        os.fchown(fd, 0, reader_gid)
        os.fchmod(fd, 0o440)
    finally:
        os.close(fd)


def copy_operator_token(source, target, reader_gid):
    if not os.path.isabs(source):
        fail("source-not-absolute")
    if source == "/nix/store" or source.startswith("/nix/store/"):
        fail("source-in-nix-store")
    reject_symlink_components(source, require_existing_file=True, check_parent_permissions=True)
    try:
        src_fd = os.open(source, os.O_RDONLY | os.O_NONBLOCK | os.O_NOFOLLOW | os.O_CLOEXEC)
    except OSError:
        fail("source-open-failed")
    try:
        st = os.fstat(src_fd)
        if not stat.S_ISREG(st.st_mode):
            fail("source-not-regular")
        if st.st_uid != 0:
            fail("source-not-root-owned")
        if stat.S_IMODE(st.st_mode) & 0o077:
            fail("source-group-or-world-permissions")
        write_fd_to_target(src_fd, target, reader_gid)
        write_source_marker(target, "operator")
    finally:
        os.close(src_fd)


def write_fd_to_target(src_fd, target, reader_gid):
    directory = os.path.dirname(target)
    prepare_target_directory(directory, reader_gid)
    tmp = os.path.join(directory, f".token.tmp.{os.getpid()}")
    try:
        dst_fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC, 0o400)
        try:
            while True:
                chunk = os.read(src_fd, 65536)
                if not chunk:
                    break
                os.write(dst_fd, chunk)
            os.fchown(dst_fd, 0, reader_gid)
            os.fchmod(dst_fd, 0o440)
            os.fsync(dst_fd)
        finally:
            os.close(dst_fd)
        os.rename(tmp, target)
    finally:
        try:
            os.unlink(tmp)
        except FileNotFoundError:
            pass


def generate_token(target, reader_gid):
    if os.path.exists(target) and read_source_marker(target) == "generated":
        validate_materialized(target, reader_gid)
        return
    directory = os.path.dirname(target)
    prepare_target_directory(directory, reader_gid)
    tmp = os.path.join(directory, f".token.tmp.{os.getpid()}")
    token = (secrets.token_urlsafe(48) + "\n").encode("ascii")
    try:
        fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC, 0o400)
        try:
            os.write(fd, token)
            os.fchown(fd, 0, reader_gid)
            os.fchmod(fd, 0o440)
            os.fsync(fd)
        finally:
            os.close(fd)
        os.rename(tmp, target)
    finally:
        try:
            os.unlink(tmp)
        except FileNotFoundError:
            pass
    validate_materialized(target, reader_gid)
    write_source_marker(target, "generated")


def marker_path(target):
    return os.path.join(os.path.dirname(target), "token.source")


def read_source_marker(target):
    path = marker_path(target)
    try:
        fd = os.open(path, os.O_RDONLY | os.O_NONBLOCK | os.O_NOFOLLOW | os.O_CLOEXEC)
    except OSError:
        return None
    try:
        data = os.read(fd, 64).decode("ascii", errors="ignore").strip()
        if data in ("generated", "operator"):
            return data
        return None
    finally:
        os.close(fd)


def write_source_marker(target, value):
    directory = os.path.dirname(target)
    tmp = os.path.join(directory, f".token.source.tmp.{os.getpid()}")
    path = marker_path(target)
    try:
        fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC, 0o600)
        try:
            os.write(fd, (value + "\n").encode("ascii"))
            os.fchown(fd, 0, 0)
            os.fchmod(fd, 0o600)
            os.fsync(fd)
        finally:
            os.close(fd)
        os.rename(tmp, path)
    finally:
        try:
            os.unlink(tmp)
        except FileNotFoundError:
            pass


def prepare_target_directory(directory, reader_gid):
    reject_symlink_components(directory, require_existing_file=False, check_parent_permissions=False)
    parent = os.path.dirname(directory)
    created_parent = False
    try:
        os.mkdir(parent, mode=0o750)
        created_parent = True
    except FileExistsError:
        pass
    try:
        parent_fd = os.open(parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
    except OSError:
        fail("target-parent-open-failed")
    try:
        st = os.fstat(parent_fd)
        if not stat.S_ISDIR(st.st_mode):
            fail("target-parent-not-directory")
        if created_parent:
            os.fchown(parent_fd, 0, 0)
            os.fchmod(parent_fd, 0o750)
    finally:
        os.close(parent_fd)
    try:
        os.mkdir(directory, mode=0o750)
    except FileExistsError:
        pass
    try:
        fd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
    except OSError:
        fail("target-directory-open-failed")
    try:
        st = os.fstat(fd)
        if not stat.S_ISDIR(st.st_mode):
            fail("target-directory-not-directory")
        os.fchown(fd, 0, reader_gid)
        os.fchmod(fd, 0o750)
    finally:
        os.close(fd)


for spec in specs:
    current_vm = spec["name"]
    target = spec["target"]
    reader_gid = int(spec.get("readerGid", 0))
    source = spec.get("source")
    if source is None:
        generate_token(target, reader_gid)
    else:
        copy_operator_token(source, target, reader_gid)
