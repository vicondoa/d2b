{ config, pkgs, lib, ... }:

let
  cfg = config.nixling;
  enabledGuestControlVms =
    lib.filterAttrs (_: vm: vm.enable && vm.guest.control.enable) cfg.vms;
  tokenSpecs = lib.mapAttrsToList (name: vm: {
    inherit name;
    source = vm.guest.control.auth.tokenFile;
    target = "${cfg.site.stateDir}/vms/${name}/guest-control/token";
  }) enabledGuestControlVms;
  tokenSpecsFile = pkgs.writeText "nixling-guest-control-token-specs.json"
    (builtins.toJSON tokenSpecs);
  tokenMaterializer = pkgs.writeText "nixling-guest-control-token-materialize.py" ''
    import json
    import os
    import secrets
    import stat
    import sys

    specs = json.load(open(sys.argv[1], encoding="utf-8"))

    def fail(message):
        print(f"nixling guest-control token: {message}", file=sys.stderr)
        sys.exit(1)

    def validate_materialized(path):
        try:
            fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
        except OSError as exc:
            fail(f"{path}: cannot open materialized token: {exc}")
        try:
            st = os.fstat(fd)
            if not stat.S_ISREG(st.st_mode):
                fail(f"{path}: materialized token is not a regular file")
            os.fchown(fd, 0, 0)
            os.fchmod(fd, 0o400)
        finally:
            os.close(fd)

    def copy_operator_token(source, target):
        if not os.path.isabs(source):
            fail(f"{source}: tokenFile must be absolute")
        if source == "/nix/store" or source.startswith("/nix/store/"):
            fail(f"{source}: tokenFile must not be under /nix/store")
        try:
            src_fd = os.open(source, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
        except OSError as exc:
            fail(f"{source}: cannot open tokenFile with O_NOFOLLOW: {exc}")
        try:
            st = os.fstat(src_fd)
            if not stat.S_ISREG(st.st_mode):
                fail(f"{source}: tokenFile is not a regular file")
            if st.st_uid != 0:
                fail(f"{source}: tokenFile must be owned by root")
            if stat.S_IMODE(st.st_mode) & 0o077:
                fail(f"{source}: tokenFile must not grant group/world permissions")
            write_fd_to_target(src_fd, target)
        finally:
            os.close(src_fd)

    def write_fd_to_target(src_fd, target):
        directory = os.path.dirname(target)
        tmp = os.path.join(directory, f".token.tmp.{os.getpid()}")
        try:
            dst_fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC, 0o400)
            try:
                while True:
                    chunk = os.read(src_fd, 65536)
                    if not chunk:
                        break
                    os.write(dst_fd, chunk)
                os.fchown(dst_fd, 0, 0)
                os.fchmod(dst_fd, 0o400)
                os.fsync(dst_fd)
            finally:
                os.close(dst_fd)
            os.rename(tmp, target)
        finally:
            try:
                os.unlink(tmp)
            except FileNotFoundError:
                pass

    def generate_token(target):
        if os.path.exists(target):
            validate_materialized(target)
            return
        directory = os.path.dirname(target)
        tmp = os.path.join(directory, f".token.tmp.{os.getpid()}")
        token = (secrets.token_urlsafe(48) + "\n").encode("ascii")
        try:
            fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC, 0o400)
            try:
                os.write(fd, token)
                os.fchown(fd, 0, 0)
                os.fchmod(fd, 0o400)
                os.fsync(fd)
            finally:
                os.close(fd)
            os.rename(tmp, target)
        finally:
            try:
                os.unlink(tmp)
            except FileNotFoundError:
                pass
        validate_materialized(target)

    for spec in specs:
        target = spec["target"]
        directory = os.path.dirname(target)
        os.makedirs(directory, mode=0o700, exist_ok=True)
        os.chown(directory, 0, 0)
        os.chmod(directory, 0o700)
        source = spec.get("source")
        if source is None:
            generate_token(target)
        else:
            copy_operator_token(source, target)
  '';
in
{
  system.activationScripts.nixlingGuestControlTokens =
    lib.stringAfter [ "users" ] ''
      ${pkgs.python3}/bin/python3 ${tokenMaterializer} ${tokenSpecsFile}
    '';
}
