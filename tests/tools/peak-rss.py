#!/usr/bin/env python3
"""Run a command with a fail-closed aggregate resident-memory ceiling."""

from __future__ import annotations

import argparse
import os
import pathlib
import signal
import subprocess
import sys
import time
from collections import deque


POLL_SECONDS = 0.05
KIB = 1024
CAUSE = (
    "Common cause: a new eval-only test or module path deep-forced a full "
    "NixOS/VM system.build.toplevel, pkgs.closureInfo, derivation "
    "realization/IFD, an equivalent VM closure, or an overly broad deep "
    "evaluation. Use a narrow attr-local fixture, shared evaluated scenario, "
    "or stubbed evaluation boundary."
)


class MeasurementError(RuntimeError):
    """Raised when aggregate resident memory cannot be measured."""


def proc_pids(root: int) -> set[int]:
    """Return the live process tree rooted at *root*."""
    children: dict[int, list[int]] = {}
    for entry in pathlib.Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            stat = (entry / "stat").read_text(encoding="ascii")
            end_name = stat.rfind(")")
            fields = stat[end_name + 2 :].split()
            ppid = int(fields[1])
            pid = int(entry.name)
        except (OSError, ValueError, IndexError):
            continue
        children.setdefault(ppid, []).append(pid)

    found = {root}
    pending = deque([root])
    while pending:
        parent = pending.popleft()
        for child in children.get(parent, []):
            if child not in found:
                found.add(child)
                pending.append(child)
    return found


def process_rss_kib(root: int) -> int | None:
    total = 0
    seen = False
    for pid in proc_pids(root):
        try:
            status = (pathlib.Path("/proc") / str(pid) / "status").read_text(
                encoding="ascii"
            )
        except OSError:
            continue
        for line in status.splitlines():
            if line.startswith("VmRSS:"):
                fields = line.split()
                if len(fields) >= 2:
                    total += int(fields[1])
                    seen = True
                break
    return total if seen else None


def cgroup_memory_file(pid: int) -> pathlib.Path | None:
    try:
        lines = (pathlib.Path("/proc") / str(pid) / "cgroup").read_text(
            encoding="ascii"
        ).splitlines()
    except OSError:
        return None
    relative = next(
        (line.split("::", 1)[1] for line in lines if line.startswith("0::")),
        None,
    )
    if relative is None:
        return None
    path = pathlib.Path("/sys/fs/cgroup") / relative.lstrip("/")
    memory = path / "memory.current"
    return memory if memory.is_file() else None


def cgroup_processes(pid: int) -> set[int] | None:
    memory_file = cgroup_memory_file(pid)
    if memory_file is None:
        return None
    try:
        return {
            int(value)
            for value in (memory_file.parent / "cgroup.procs")
            .read_text(encoding="ascii")
            .split()
        }
    except (OSError, ValueError):
        return None


def cgroup_current_kib(pid: int) -> int | None:
    memory_file = cgroup_memory_file(pid)
    if memory_file is None:
        return None
    try:
        return int(memory_file.read_text(encoding="ascii").strip()) // KIB
    except (OSError, ValueError):
        return None


def kill_group(process: subprocess.Popen[bytes]) -> None:
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        return
    try:
        process.wait(timeout=2)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait()


def format_kib(value: int) -> str:
    return f"{value} KiB ({value / 1024:.1f} MiB)"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run a command with an aggregate process-tree/cgroup RSS ceiling."
    )
    parser.add_argument("--lane", required=True)
    parser.add_argument("--max-kib", required=True, type=int)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.max_kib <= 0:
        parser.error("--max-kib must be positive")
    if not args.command:
        parser.error("a command is required after --")
    if args.command[0] == "--":
        args.command = args.command[1:]
    if not args.command:
        parser.error("a command is required after --")
    return args


def main() -> int:
    args = parse_args()
    baseline_cgroup = cgroup_current_kib(os.getpid())
    baseline_cgroup_processes = cgroup_processes(os.getpid())
    # A user service scope commonly contains unrelated sibling agents. Its
    # memory.current is not attributable to this command, so use the complete
    # process tree unless the scope was already dedicated to this helper.
    cgroup_isolated = (
        baseline_cgroup_processes is not None
        and len(baseline_cgroup_processes) <= 2
    )
    try:
        process = subprocess.Popen(
            args.command,
            start_new_session=True,
        )
    except OSError as error:
        print(
            f"{args.lane} peak RSS guard could not start command: {error}",
            file=sys.stderr,
        )
        return 125

    peak_tree: int | None = None
    peak_cgroup_delta: int | None = None
    exceeded = False
    while process.poll() is None:
        tree = process_rss_kib(process.pid)
        if tree is not None:
            peak_tree = max(peak_tree or 0, tree)
        cgroup = cgroup_current_kib(process.pid)
        if cgroup_isolated and cgroup is not None and baseline_cgroup is not None:
            peak_cgroup_delta = max(
                peak_cgroup_delta or 0,
                max(0, cgroup - baseline_cgroup),
            )
        observed = max(peak_tree or 0, peak_cgroup_delta or 0)
        if observed > args.max_kib:
            exceeded = True
            kill_group(process)
            break
        time.sleep(POLL_SECONDS)

    if process.poll() is None:
        process.wait()
    # A worker launcher can leave a short-lived descendant behind after its
    # parent exits. Reap the process group before the outer runner starts its
    # next shard, otherwise a later aggregate tree would retain that worker.
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    final_tree = process_rss_kib(process.pid)
    if final_tree is not None:
        peak_tree = max(peak_tree or 0, final_tree)
    if peak_tree is None and peak_cgroup_delta is None:
        print(
            f"{args.lane} peak RSS guard could not obtain process-tree or cgroup "
            f"resident memory; refusing to report an unmeasured pass. {CAUSE}",
            file=sys.stderr,
        )
        return 125

    observed = max(peak_tree or 0, peak_cgroup_delta or 0)
    method = "process-tree"
    if cgroup_isolated and peak_cgroup_delta is not None:
        method += "+baseline-adjusted-cgroup"
    print(
        f"{args.lane} peak RSS: observed {format_kib(observed)}, "
        f"maximum {format_kib(args.max_kib)}, "
        f"tree={format_kib(peak_tree or 0)}, "
        f"cgroup_delta={format_kib(peak_cgroup_delta or 0)}, method={method}",
        file=sys.stderr,
    )
    if exceeded or observed > args.max_kib:
        print(
            f"{args.lane} peak RSS guard failed: observed {format_kib(observed)} "
            f"> maximum {format_kib(args.max_kib)}. {CAUSE}",
            file=sys.stderr,
        )
        return 125
    return process.returncode or 0


if __name__ == "__main__":
    raise SystemExit(main())
