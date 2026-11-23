#!/usr/bin/env python3
"""macOS process resources, by harness `phase`, against a temporary library.

python3 tools/perf/resource_probe.py script.txt source-library output-dir
Requires a release devtools build. `idle` leaves drawing to the native event
loop; `wait` also draws invalidated frames. CPU 100% means one logical core.
The child PID comes from Popen, never a process-name search. Only that child
is terminated on timeout. Library and local caches are isolated under output.
"""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import selectors
import shutil
import subprocess
import sys
import time


FIELDS = """user_time system_time pkg_idle_wkups interrupt_wkups pageins
wired_size resident_size phys_footprint proc_start_abstime proc_exit_abstime
child_user_time child_system_time child_pkg_idle_wkups child_interrupt_wkups
child_pageins child_elapsed_abstime diskio_bytesread diskio_byteswritten
cpu_time_qos_default cpu_time_qos_maintenance cpu_time_qos_background
cpu_time_qos_utility cpu_time_qos_legacy cpu_time_qos_user_initiated
cpu_time_qos_user_interactive billed_system_time serviced_system_time
logical_writes lifetime_max_phys_footprint instructions cycles billed_energy
serviced_energy interval_max_phys_footprint runnable_time""".split()


class Usage(ctypes.Structure):
    _fields_ = [("uuid", ctypes.c_uint8 * 16)] + [(k, ctypes.c_uint64) for k in FIELDS]


class Timebase(ctypes.Structure):
    _fields_ = [("numer", ctypes.c_uint32), ("denom", ctypes.c_uint32)]


def snapshot_files(roots):
    out = {}
    for root in roots:
        if not root.exists():
            continue
        for path in root.rglob("*"):
            if path.is_file():
                try:
                    stat = path.stat()
                    out[str(path)] = (stat.st_ino, stat.st_mtime_ns, stat.st_size)
                except FileNotFoundError:
                    pass
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("script", type=Path)
    parser.add_argument("library", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--binary", type=Path, default=Path(__file__).resolve().parents[2] / "target/release/quiet_md")
    parser.add_argument("--timeout", type=float, default=300)
    parser.add_argument("--watch", action="append", default=[], help="library-relative file to track for identical rewrites")
    parser.add_argument("--no-blink", action="store_true", help="diagnostic control: disable caret blinking")
    parser.add_argument("--watch-cache", action="store_true", help="also hash replacements of the local parsed cache")
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error("proc_pid_rusage requires macOS")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    library = output / "lib"
    if library.exists():
        parser.error("output/lib already exists; choose a fresh output directory")
    shutil.copytree(args.library, library)
    script = output / "steps.txt"
    script.write_text(args.script.read_text() + "\nphase shutdown\nquit\n")
    binary = args.binary.resolve()
    if b"devtools: cannot read" not in binary.read_bytes():
        parser.error("binary needs --features devtools")
    libproc = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
    libproc.proc_pid_rusage.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p]
    libproc.proc_pid_rusage.restype = ctypes.c_int
    timebase = Timebase()
    ctypes.CDLL("/usr/lib/libSystem.B.dylib").mach_timebase_info(ctypes.byref(timebase))
    seconds_per_tick = timebase.numer / timebase.denom / 1e9
    env = {**os.environ, "QUIET_MD_HOME": str(library), "TMPDIR": str(output),
           "QUIET_MD_LOG_DIR": str(output / "app-logs"), "FDL_SCRIPT": str(script), "FDL_SHOTS": str(output)}
    for key in ("FDL_NOPARK", "FDL_TRACE", "FDL_NOBLINK", "QUIET_MD_ONBOARDING"):
        env.pop(key, None)
    if args.no_blink:
        env["FDL_NOBLINK"] = "1"
    start = time.monotonic()
    process = subprocess.Popen([str(binary)], cwd=output, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    (output / "process.json").write_text(json.dumps({"pid": process.pid, "binary": str(binary)}))
    rows, phases, lines, pending = [], [], [], b""
    phase = "startup"
    roots = [library, output / "quiet-md-local"]
    files_before = snapshot_files(roots)
    watch = [library / "quiet.json"]
    watch += [library / name for name in args.watch] if args.watch else sorted((library / "Notes").glob("*"))[:4]
    stamps, hashes, writes = {}, {}, []
    for path in watch:
        if path.is_file():
            stat = path.stat()
            stamps[path] = (stat.st_ino, stat.st_mtime_ns, stat.st_size)
            hashes[path] = hashlib.sha256(path.read_bytes()).hexdigest()

    def watch_files():
        if args.watch_cache:
            for path in (output / "quiet-md-local/list").rglob("*.json"):
                if path not in watch:
                    watch.append(path)
        for path in watch:
            try:
                stat = path.stat()
                stamp = (stat.st_ino, stat.st_mtime_ns, stat.st_size)
                if stamps.get(path) == stamp:
                    continue
                digest = hashlib.sha256(path.read_bytes()).hexdigest()
                writes.append({"seconds": round(time.monotonic() - start, 4), "phase": phase,
                               "file": os.path.relpath(path, library), "bytes": stat.st_size,
                               "identical": hashes.get(path) == digest})
                stamps[path], hashes[path] = stamp, digest
            except FileNotFoundError:
                pass

    def sample():
        usage = Usage()
        if libproc.proc_pid_rusage(process.pid, 4, ctypes.byref(usage)):
            return None
        row = {"seconds": round(time.monotonic() - start, 4), "phase": phase,
               **{key: getattr(usage, key) for key in FIELDS}}
        rows.append(row)
        return row

    def finish_phase():
        nonlocal files_before
        after = snapshot_files(roots)
        changed = [str(Path(p).relative_to(output)) for p, stamp in after.items() if files_before.get(p) != stamp]
        removed = [str(Path(p).relative_to(output)) for p in files_before if p not in after]
        group = [r for r in rows if r["phase"] == phase]
        if len(group) >= 2:
            first, last = group[0], group[-1]
            seconds = last["seconds"] - first["seconds"]
            cpu = ((last["user_time"] + last["system_time"]) - (first["user_time"] + first["system_time"])) * seconds_per_tick
            summary = {"phase": phase, "seconds": round(seconds, 3), "cpu_percent": round(100 * cpu / max(seconds, .001), 2),
                       "footprint_start_mib": round(first["phys_footprint"] / 2**20, 2),
                       "footprint_end_mib": round(last["phys_footprint"] / 2**20, 2),
                       "footprint_peak_mib": round(max(r["phys_footprint"] for r in group) / 2**20, 2),
                       "lifetime_footprint_peak_mib": round(max(r["lifetime_max_phys_footprint"] for r in group) / 2**20, 2),
                       "rss_peak_mib": round(max(r["resident_size"] for r in group) / 2**20, 2),
                       "logical_write_bytes": last["logical_writes"] - first["logical_writes"],
                       "disk_write_bytes": last["diskio_byteswritten"] - first["diskio_byteswritten"],
                       "disk_read_bytes": last["diskio_bytesread"] - first["diskio_bytesread"],
                       "idle_wakeups": last["pkg_idle_wkups"] - first["pkg_idle_wkups"],
                       "observed_writes": [w for w in writes if w["phase"] == phase],
                       "changed_files": changed, "removed_files": removed}
            phases.append(summary)
            print(json.dumps(summary, ensure_ascii=False), flush=True)
        files_before = after

    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ)
    timed_out = False
    try:
        with (output / "log").open("wb") as log:
            while process.poll() is None or selector.get_map():
                if time.monotonic() - start > args.timeout and process.poll() is None:
                    timed_out = True
                    try:
                        subprocess.run(["sample", str(process.pid), "2", "-file", str(output / "timeout-sample.txt")],
                                       capture_output=True, timeout=10)
                    except subprocess.TimeoutExpired:
                        pass
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                    break
                for key, _ in selector.select(.1):
                    data = os.read(key.fileobj.fileno(), 65536)
                    if not data:
                        selector.unregister(key.fileobj)
                        continue
                    log.write(data)
                    log.flush()
                    pending += data
                    while b"\n" in pending:
                        line, pending = pending.split(b"\n", 1)
                        text = line.decode(errors="replace")
                        lines.append(text)
                        if text.startswith("devtools: phase "):
                            boundary = sample()
                            finish_phase()
                            phase = text.removeprefix("devtools: phase ")
                            if boundary:
                                rows.append({**boundary, "phase": phase})
                sample()
                watch_files()
    finally:
        if process.poll() is None:
            process.kill()
        process.wait()
        selector.close()
    finish_phase()
    (output / "samples.jsonl").write_text("".join(json.dumps(r) + "\n" for r in rows))
    (output / "file-writes.jsonl").write_text("".join(json.dumps(r) + "\n" for r in writes))
    result = {"binary": str(binary), "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
              "cpu_seconds_per_tick": seconds_per_tick,
              "caret_blink": not args.no_blink,
              "pid": process.pid, "returncode": process.returncode, "timed_out": timed_out,
              "elapsed_seconds": round(time.monotonic() - start, 3), "phases": phases,
              "benchmarks": [line for line in lines if line.startswith("bench ")]}
    (output / "resources.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
    print(f"Resources: {output / 'resources.json'}", flush=True)
    if timed_out or process.returncode or any("panicked at" in line for line in lines):
        sys.exit(1)


if __name__ == "__main__":
    main()
