#!/usr/bin/env python3

import argparse
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tomllib


DEPENDENCY_TABLES = {"dependencies", "dev-dependencies", "build-dependencies"}
COPY_RE = re.compile(
    r"cp -r \$\{\./packages/([^}\s]+)\} \$out/packages/([^\s]+)"
)
MANIFEST_COPY_RE = re.compile(
    r"cp\s+\$\{\./tests/fixtures/guest-rust-workspace/([^}]+)\}"
    r"\s+\\?\s*\$out/packages/(?:([^/\s]+)/)?Cargo\.toml"
)


class DriftError(Exception):
    pass


def load_toml(path: Path) -> dict:
    try:
        with path.open("rb") as handle:
            return tomllib.load(handle)
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise DriftError(f"cannot parse {path}: {error}") from error


def dependency_tables(value: object, prefix: str = ""):
    if not isinstance(value, dict):
        return
    for key, child in value.items():
        location = f"{prefix}.{key}" if prefix else key
        if key in DEPENDENCY_TABLES and isinstance(child, dict):
            yield location, child
        elif isinstance(child, dict):
            yield from dependency_tables(child, location)


def dependency_name(alias: str, specification: object) -> str:
    if isinstance(specification, dict):
        package = specification.get("package")
        if isinstance(package, str):
            return package
    return alias


def require_lock_package(
    packages_by_name: dict[str, list[dict]], name: str, origin: str
) -> None:
    if name not in packages_by_name:
        raise DriftError(
            f"packages/Cargo.guest.lock is missing package '{name}' required by {origin}"
        )


def copied_crates(flake_text: str) -> list[str]:
    if flake_text.count("mkGuestRustPackagesSrc = pkgs:") != 1:
        raise DriftError(
            "flake.nix must define exactly one mkGuestRustPackagesSrc constructor"
        )
    if flake_text.count("guestRustPackagesSrc = mkGuestRustPackagesSrc pkgs;") != 2:
        raise DriftError(
            "flake.nix packages and checks must both use mkGuestRustPackagesSrc"
        )

    copies = COPY_RE.findall(flake_text)
    if not copies:
        raise DriftError("mkGuestRustPackagesSrc copies no guest crates")
    for source, destination in copies:
        if source != destination:
            raise DriftError(
                "guest crate copy changes the crate name: "
                f"packages/{source} -> packages/{destination}"
            )
    crates = [source for source, _ in copies]
    duplicates = sorted({crate for crate in crates if crates.count(crate) > 1})
    if duplicates:
        raise DriftError(
            "guest constructor copies crates more than once: " + ", ".join(duplicates)
        )
    return crates


def manifest_overrides(flake_text: str) -> dict[str, str]:
    copies = MANIFEST_COPY_RE.findall(flake_text)
    overrides: dict[str, str] = {}
    root_seen = False
    for fixture_name, crate in copies:
        if crate:
            overrides[crate] = fixture_name
        elif fixture_name == "Cargo.toml":
            root_seen = True
    if not root_seen:
        raise DriftError(
            "mkGuestRustPackagesSrc does not install the canonical guest Cargo.toml"
        )
    return overrides


def validate_inherited_dependencies(
    root: Path,
    crates: list[str],
    guest_workspace_dependencies: dict,
) -> None:
    for crate in crates:
        manifest_path = root / "packages" / crate / "Cargo.toml"
        manifest = load_toml(manifest_path)
        for table_name, table in dependency_tables(manifest):
            for dependency, specification in table.items():
                if (
                    isinstance(specification, dict)
                    and specification.get("workspace") is True
                    and dependency not in guest_workspace_dependencies
                ):
                    raise DriftError(
                        "guest workspace dependency drift: "
                        f"packages/{crate}/Cargo.toml [{table_name}] inherits "
                        f"'{dependency}', but "
                        "tests/fixtures/guest-rust-workspace/Cargo.toml "
                        "[workspace.dependencies] does not define it"
                    )


def validate_lock(
    root: Path,
    crates: list[str],
    overrides: dict[str, str],
    guest_manifest: dict,
) -> None:
    lock_path = root / "packages" / "Cargo.guest.lock"
    lock = load_toml(lock_path)
    packages = lock.get("package")
    if not isinstance(packages, list) or not packages:
        raise DriftError(f"{lock_path} contains no [[package]] entries")

    packages_by_name: dict[str, list[dict]] = {}
    exact_packages: set[tuple[object, object, object]] = set()
    for package in packages:
        if not isinstance(package, dict):
            raise DriftError(f"{lock_path} has a malformed [[package]] entry")
        name = package.get("name")
        version = package.get("version")
        if not isinstance(name, str) or not isinstance(version, str):
            raise DriftError(f"{lock_path} has a package without a name and version")
        exact = (name, version, package.get("source"))
        if exact in exact_packages:
            raise DriftError(
                f"{lock_path} repeats package '{name}' version '{version}'"
            )
        exact_packages.add(exact)
        packages_by_name.setdefault(name, []).append(package)

    guest_dependencies = guest_manifest["workspace"].get("dependencies", {})
    fixture_root = root / "tests" / "fixtures" / "guest-rust-workspace"
    for crate in crates:
        source = root / "packages" / crate / "Cargo.toml"
        if crate in overrides:
            source = fixture_root / overrides[crate]
        manifest = load_toml(source)
        package = manifest.get("package", {})
        package_name = package.get("name")
        if not isinstance(package_name, str):
            raise DriftError(f"{source} has no package.name")
        require_lock_package(packages_by_name, package_name, source.relative_to(root).as_posix())

        for table_name, table in dependency_tables(manifest):
            for alias, specification in table.items():
                effective_specification = specification
                if (
                    isinstance(specification, dict)
                    and specification.get("workspace") is True
                ):
                    effective_specification = guest_dependencies.get(alias)
                    if effective_specification is None:
                        continue
                name = dependency_name(alias, effective_specification)
                require_lock_package(
                    packages_by_name,
                    name,
                    f"{source.relative_to(root).as_posix()} [{table_name}]",
                )

    versions = {
        (package["name"], package["version"])
        for package in packages
        if isinstance(package, dict)
    }
    for package in packages:
        origin = f"{package['name']} {package['version']}"
        dependencies = package.get("dependencies", [])
        if not isinstance(dependencies, list):
            raise DriftError(
                f"{lock_path} package '{origin}' has malformed dependencies"
            )
        for dependency in dependencies:
            if not isinstance(dependency, str):
                raise DriftError(
                    f"{lock_path} package '{origin}' has a non-string dependency"
                )
            fields = dependency.split(" ", 2)
            name = fields[0]
            if len(fields) == 1:
                require_lock_package(packages_by_name, name, f"lock package {origin}")
            elif (name, fields[1]) not in versions:
                raise DriftError(
                    "packages/Cargo.guest.lock is missing resolved package "
                    f"'{name} {fields[1]}' required by lock package {origin}"
                )


def validate_with_cargo(
    root: Path,
    cargo: str,
    crates: list[str],
    overrides: dict[str, str],
) -> None:
    scratch_root = root / ".scratch"
    scratch_root.mkdir(exist_ok=True)
    workspace = scratch_root / f"guest-workspace-drift-{os.getpid()}"
    if workspace.exists():
        shutil.rmtree(workspace)
    workspace.mkdir()

    fixture_root = root / "tests" / "fixtures" / "guest-rust-workspace"
    try:
        for crate in crates:
            shutil.copytree(root / "packages" / crate, workspace / crate, symlinks=True)
            override = overrides.get(crate)
            if override:
                shutil.copyfile(
                    fixture_root / override, workspace / crate / "Cargo.toml"
                )
        shutil.copyfile(fixture_root / "Cargo.toml", workspace / "Cargo.toml")
        shutil.copyfile(
            root / "packages" / "Cargo.guest.lock", workspace / "Cargo.lock"
        )
        result = subprocess.run(
            [
                cargo,
                "metadata",
                "--manifest-path",
                str(workspace / "Cargo.toml"),
                "--format-version",
                "1",
                "--locked",
                "--no-deps",
            ],
            cwd=root,
            check=False,
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            details = result.stderr.strip() or result.stdout.strip()
            raise DriftError(
                "guest workspace does not resolve with packages/Cargo.guest.lock "
                f"under 'cargo metadata --locked --no-deps':\n{details}"
            )
    finally:
        shutil.rmtree(workspace, ignore_errors=True)


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Check the flake guest workspace mirror and lock for drift."
    )
    parser.add_argument(
        "--root",
        type=Path,
        default=Path(__file__).resolve().parents[2],
        help="repository root",
    )
    parser.add_argument(
        "--cargo",
        default=os.environ.get("CARGO", "cargo"),
        help="cargo executable",
    )
    args = parser.parse_args()
    root = args.root.resolve()

    try:
        flake_text = (root / "flake.nix").read_text(encoding="utf-8")
        crates = copied_crates(flake_text)
        overrides = manifest_overrides(flake_text)
        fixture_root = root / "tests" / "fixtures" / "guest-rust-workspace"
        guest_manifest = load_toml(fixture_root / "Cargo.toml")
        members = guest_manifest.get("workspace", {}).get("members")
        if not isinstance(members, list) or not all(
            isinstance(member, str) for member in members
        ):
            raise DriftError(
                "guest workspace Cargo.toml must declare literal workspace.members"
            )
        if set(members) != set(crates):
            raise DriftError(
                "guest constructor copies and workspace members differ: "
                f"copied={sorted(crates)}, members={sorted(members)}"
            )

        expected_override_files = {
            path.name
            for path in fixture_root.glob("*.Cargo.toml")
            if path.name != "Cargo.toml"
        }
        if set(overrides.values()) != expected_override_files:
            raise DriftError(
                "guest manifest overrides installed by flake.nix differ from fixtures: "
                f"installed={sorted(overrides.values())}, "
                f"fixtures={sorted(expected_override_files)}"
            )
        if not set(overrides).issubset(crates):
            raise DriftError(
                "guest manifest override targets a crate the constructor does not copy"
            )

        guest_dependencies = guest_manifest.get("workspace", {}).get("dependencies")
        if not isinstance(guest_dependencies, dict):
            raise DriftError(
                "guest workspace Cargo.toml has no [workspace.dependencies] table"
            )
        validate_inherited_dependencies(root, crates, guest_dependencies)
        validate_lock(root, crates, overrides, guest_manifest)
        validate_with_cargo(root, args.cargo, crates, overrides)
    except (DriftError, OSError) as error:
        print(f"FAIL: guest-workspace-drift: {error}", file=sys.stderr)
        return 1

    print(
        "guest-workspace-drift OK "
        f"({len(crates)} mirrored crates, inherited dependencies and lock verified)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
