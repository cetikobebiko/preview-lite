#!/usr/bin/env python3
"""
Memory guard for measurement sessions on macOS.

Watches the running app (release or `tauri dev` build), its WebKit helper processes
(WebContent, GPU, Networking) and its PDFium workers. Records their physical footprint
(the "Memory" column of Activity Monitor) to a CSV file, and kills the app before the
machine runs out of memory: on 2026-10-13 the WebContent process grew to 38 GB during a
pinch zoom and macOS had to be force-restarted.

macOS does not say which app a WebKit helper belongs to, so helpers that started after
the app are counted as the app's. Close Safari and other WebKit apps while measuring.

Usage (run in its own terminal, then start the app any way you like):
  python3 scripts/memory-guard.py
  python3 scripts/memory-guard.py --limit-mb 3000 --csv logs/zoom-test.csv
Stops when the app exits. Standard library only.
"""

import argparse
import ctypes
import os
import signal
import subprocess
import sys
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
APP_NAME = "preview-alternative"
WEBKIT_PREFIX = "com.apple.WebKit."
RUSAGE_INFO_V4 = 4


class RusageInfo(ctypes.Structure):
    # Leading fields of struct rusage_info_v4 (<sys/resource.h>); the tail is padding.
    _fields_ = [("ri_uuid", ctypes.c_uint8 * 16)] + [
        (name, ctypes.c_uint64)
        for name in (
            "ri_user_time", "ri_system_time", "ri_pkg_idle_wkups", "ri_interrupt_wkups",
            "ri_pageins", "ri_wired_size", "ri_resident_size", "ri_phys_footprint",
        )
    ] + [("ri_rest", ctypes.c_uint64 * 40)]


_libc = ctypes.CDLL("/usr/lib/libSystem.B.dylib")
_proc_pid_rusage = _libc.proc_pid_rusage
_proc_pid_rusage.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p]
_proc_pid_rusage.restype = ctypes.c_int


def footprint_mb(pid):
    info = RusageInfo()
    if _proc_pid_rusage(pid, RUSAGE_INFO_V4, ctypes.byref(info)) != 0:
        return None
    return info.ri_phys_footprint / 1048576


def parse_etime(text):
    """ps etime ([[dd-]hh:]mm:ss) -> seconds."""
    days, _, rest = text.strip().rpartition("-")
    parts = [int(part) for part in rest.split(":")]
    while len(parts) < 3:
        parts.insert(0, 0)
    hours, minutes, seconds = parts
    return (int(days) if days else 0) * 86400 + hours * 3600 + minutes * 60 + seconds


def list_processes():
    out = subprocess.run(["ps", "-axo", "pid=,etime=,args="], capture_output=True, text=True).stdout
    now = time.time()
    processes = []
    for line in out.splitlines():
        fields = line.strip().split(None, 2)
        if len(fields) < 3:
            continue
        pid, etime, args = fields
        processes.append({"pid": int(pid), "started": now - parse_etime(etime), "args": args})
    return processes


def classify(processes, app_name=APP_NAME):
    """Main app, its workers and the WebKit helpers that started after it."""
    apps = [p for p in processes if Path(p["args"].split(" ")[0]).name == app_name]
    main = [p for p in apps if "--run-" not in p["args"]]
    if not main:
        return None
    app = min(main, key=lambda p: p["started"])
    workers = [p for p in apps if p is not app]
    helpers = {}
    for p in processes:
        executable = Path(p["args"].split(" ")[0]).name
        # etime has one-second resolution.
        if executable.startswith(WEBKIT_PREFIX) and p["started"] >= app["started"] - 1:
            kind = executable[len(WEBKIT_PREFIX):]
            helpers.setdefault(kind, []).append(p)
    return app, workers, helpers


def kill(pids):
    for pid in pids:
        try:
            os.kill(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass


def main(argv=None):
    parser = argparse.ArgumentParser(description="Kill the app before WebKit memory takes the machine down.")
    parser.add_argument("--limit-mb", type=float, default=4096, help="kill when WebContent or the app total exceeds this (default 4096)")
    parser.add_argument("--interval", type=float, default=0.5, help="seconds between samples (default 0.5)")
    parser.add_argument("--csv", type=Path, help="sample file (default logs/memory-guard-<time>.csv)")
    parser.add_argument("--app-name", default=APP_NAME, help=f"executable name to watch (default {APP_NAME})")
    parser.add_argument("--wait-minutes", type=float, default=10, help="give up if the app has not started by then (default 10)")
    args = parser.parse_args(argv)

    csv_path = args.csv or REPO_ROOT / "logs" / f"memory-guard-{time.strftime('%Y%m%d-%H%M%S')}.csv"
    csv_path.parent.mkdir(parents=True, exist_ok=True)
    print(f"memory-guard: waiting for {args.app_name} (limit {args.limit_mb:.0f} MB) ...", flush=True)

    seen_app = None
    give_up_at = time.time() + args.wait_minutes * 60
    peak = {}
    last_print = 0.0
    with open(csv_path, "w") as csv:
        csv.write("time,app_mb,webcontent_mb,gpu_mb,networking_mb,workers_mb,total_mb\n")
        while True:
            found = classify(list_processes(), args.app_name)
            if found is None:
                if seen_app is not None:
                    print("memory-guard: app exited.", flush=True)
                    break
                if time.time() > give_up_at:
                    print(f"memory-guard: {args.app_name} did not start within {args.wait_minutes:g} minutes; exiting.", flush=True)
                    break
                time.sleep(args.interval)
                continue
            app, workers, helpers = found
            if seen_app != app["pid"]:
                seen_app = app["pid"]
                print(f"memory-guard: watching pid {seen_app}, samples -> {csv_path}", flush=True)

            def total(group):
                return sum(footprint_mb(p["pid"]) or 0 for p in group)

            row = {
                "app": footprint_mb(app["pid"]) or 0,
                "webcontent": total(helpers.get("WebContent", [])),
                "gpu": total(helpers.get("GPU", [])),
                "networking": total(helpers.get("Networking", [])),
                "workers": total(workers),
            }
            row["total"] = sum(row.values())
            for key, value in row.items():
                peak[key] = max(peak.get(key, 0), value)
            csv.write(time.strftime("%H:%M:%S") + "," + ",".join(f"{row[k]:.0f}" for k in
                      ("app", "webcontent", "gpu", "networking", "workers", "total")) + "\n")
            csv.flush()

            now = time.time()
            if now - last_print >= 2:
                last_print = now
                print(f"{time.strftime('%H:%M:%S')}  WebContent {row['webcontent']:6.0f} MB  GPU {row['gpu']:5.0f} MB  "
                      f"app {row['app']:5.0f} MB  workers {row['workers']:5.0f} MB  total {row['total']:6.0f} MB", flush=True)

            if row["webcontent"] > args.limit_mb or row["total"] > args.limit_mb * 1.5:
                victims = [app["pid"]] + [p["pid"] for p in workers] + [p["pid"] for group in helpers.values() for p in group]
                kill(victims)
                print(f"\a\nmemory-guard: LIMIT EXCEEDED (WebContent {row['webcontent']:.0f} MB, total {row['total']:.0f} MB) "
                      f"at {time.strftime('%H:%M:%S')}; killed {victims}", flush=True)
                break
            time.sleep(args.interval)

    print("memory-guard: peaks " + ", ".join(f"{key} {value:.0f} MB" for key, value in peak.items()), flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
