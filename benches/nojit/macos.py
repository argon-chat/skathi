#!/usr/bin/env python3
"""Measure published macOS apps. For showcase, call macos-scroll.ts after mounting."""

import argparse
from collections import deque
import json
import math
import os
from pathlib import Path
import platform
import plistlib
import queue
import re
import statistics
import subprocess
import sys
import tempfile
import threading
import time


APP_NAMES = ("minimal", "showcase")
VARIANTS = ("jit", "aot", "nojit")
FIRST_FRAME = re.compile(r"\[sk-perf\] boot->first-frame ([\d.]+)ms")


def executable(bundle):
    with (bundle / "Contents/Info.plist").open("rb") as stream:
        name = plistlib.load(stream)["CFBundleExecutable"]
    return bundle / "Contents/MacOS" / name


def bundle_bytes(bundle):
    logical = sum(path.stat().st_size for path in bundle.rglob("*") if path.is_file() and not path.is_symlink())
    allocated = subprocess.check_output(["/usr/bin/du", "-sk", str(bundle)], text=True).split()[0]
    return {"logicalBytes": logical, "allocatedBytes": int(allocated) * 1024}


def memory(pid):
    rss = subprocess.check_output(["/bin/ps", "-o", "rss=", "-p", str(pid)], text=True)
    footprint = subprocess.check_output(
        ["/usr/bin/footprint", "--noCategories", "-f", "bytes", "--pid", str(pid)], text=True,
    )
    physical = re.search(r"^\s*phys_footprint:\s*(\d+) B\s*$", footprint, re.MULTILINE)
    if physical is None:
        raise RuntimeError(f"footprint did not report phys_footprint for PID {pid}\n{footprint}")
    return {"rssBytes": int(rss) * 1024, "physicalFootprintBytes": int(physical[1])}


def summarized(sample, runs):
    return {
        key: summarized(value, [run[key] for run in runs]) if isinstance(value, dict)
        else statistics.median(run[key] for run in runs)
        for key, value in sample.items() if not isinstance(value, list)
    }


def capture(exe, variant, profile, idle_seconds, timeout, *, scroll=False):
    env = os.environ.copy()
    env.update(
        SK_DATA_DIR=str(profile),
        SK_APP_ENGINE="jit" if variant == "jit" else "auto",
        SK_DEVTOOLS="0",
        SK_DIALOGS="scripted",
        SK_LAUNCH_RAISE="off",
        SK_OCCLUSION="0",
        SK_PERF="1",
        SK_SCENARIO="macos-scroll" if scroll else "",
        SK_SINGLE_INSTANCE="0",
        SK_WINDOW_FOCUS="never",
    )
    lines = queue.Queue()
    recent = deque(maxlen=30)
    started = time.monotonic()
    with subprocess.Popen(
        [str(exe)], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, errors="replace", env=env,
    ) as child:
        def read_log():
            for line in child.stdout:
                lines.put((time.monotonic(), line.rstrip()))
            lines.put((time.monotonic(), None))

        reader = threading.Thread(target=read_log, daemon=True)
        reader.start()
        aot_active = False
        native_entry_completed = False
        try:
            while True:
                remaining = started + timeout - time.monotonic()
                if remaining <= 0:
                    raise queue.Empty
                stamp, line = lines.get(timeout=remaining)
                if line is None:
                    raise RuntimeError(f"app exited before reporting a result ({child.wait()})")
                recent.append(line)
                aot_active |= "[native-aot] selected application image" in line
                native_entry_completed |= "[native-aot] application native entry completed" in line
                boot = FIRST_FRAME.search(line)
                if (not scroll and boot) or (scroll and "SKMACSCROLL " in line):
                    if aot_active != (variant != "jit") or (aot_active and not native_entry_completed):
                        raise RuntimeError("the app did not run under the expected engine")
                if not scroll and boot:
                    after_frame = memory(child.pid)
                    time.sleep(idle_seconds)
                    return {
                        "spawnToFirstFrameMs": (stamp - started) * 1000,
                        "bootToFirstFrameMs": float(boot[1]),
                        "afterFirstFrame": after_frame,
                        "idle": memory(child.pid),
                    }
                if scroll and "SKMACSCROLL " in line:
                    sample = json.loads(line.split("SKMACSCROLL ", 1)[1])
                    if "error" in sample:
                        raise RuntimeError(sample["error"])
                    intervals = sorted(sample["intervals"])
                    if len(intervals) != 600 or not all(value > 0 for value in intervals) or sample["moved"] <= 0:
                        raise RuntimeError("expected 600 positive frame intervals and a scroll position that changed")
                    return {
                        "frameP50Ms": statistics.median(intervals),
                        "frameP95Ms": intervals[math.ceil(0.95 * len(intervals)) - 1],
                        "framesOver33Ms": sum(value > 33.33 for value in intervals),
                        "scrollPixels": sample["moved"],
                        "intervalsMs": sample["intervals"],
                    }
        except (RuntimeError, subprocess.CalledProcessError, queue.Empty) as error:
            reason = f"no result within {timeout}s" if isinstance(error, queue.Empty) else str(error)
            raise RuntimeError(f"{exe} ({variant}): {reason}\n" + "\n".join(recent)) from error
        finally:
            if child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait()
            reader.join()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in APP_NAMES:
        for variant in VARIANTS:
            parser.add_argument(f"--{name}-{variant}", type=Path, required=True)
    parser.add_argument("--runs", type=int, default=7, help="warm and scroll runs per app and engine")
    parser.add_argument("--idle-seconds", type=float, default=5)
    parser.add_argument("--timeout", type=float, default=60)
    parser.add_argument("--out", type=Path, required=True, help="JSON report with individual samples")
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error("this benchmark requires macOS")
    if args.runs < 1 or args.idle_seconds <= 0 or args.timeout <= 0:
        parser.error("--runs, --idle-seconds, and --timeout must be positive")

    report = {
        "macosVersion": platform.mac_ver()[0], "architecture": platform.machine(),
        "hardwareModel": subprocess.check_output(["/usr/sbin/sysctl", "-n", "hw.model"], text=True).strip(),
        "cpu": subprocess.check_output(["/usr/sbin/sysctl", "-n", "machdep.cpu.brand_string"], text=True).strip(),
        "memoryBytes": int(subprocess.check_output(["/usr/sbin/sysctl", "-n", "hw.memsize"], text=True)),
        "runs": args.runs, "idleSeconds": args.idle_seconds, "apps": {},
    }
    bundles = {
        (name, variant): getattr(args, f"{name}_{variant}").resolve()
        for name in APP_NAMES for variant in VARIANTS
    }
    exes = {key: executable(bundle) for key, bundle in bundles.items()}
    for name in APP_NAMES:
        report["apps"][name] = {}
        for variant in VARIANTS:
            bundle = bundles[name, variant]
            report["apps"][name][variant] = {
                "bundle": str(bundle), "bundleSize": bundle_bytes(bundle), "warm": [], "scroll": [],
            }

    def save():
        args.out.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")

    with tempfile.TemporaryDirectory(prefix="sk-macos-bench-") as folder:
        for name in APP_NAMES:
            app_results = report["apps"][name]
            for run in range(args.runs + 1):
                for variant in VARIANTS if run % 2 == 0 else reversed(VARIANTS):
                    profile = Path(folder) / f"{name}-{variant}"
                    sample = capture(exes[name, variant], variant, profile, args.idle_seconds, args.timeout)
                    if run == 0:
                        app_results[variant]["firstLaunch"] = sample
                    else:
                        app_results[variant]["warm"].append(sample)
                    save()
            if name == "showcase":
                for run in range(args.runs):
                    for variant in VARIANTS if run % 2 == 0 else reversed(VARIANTS):
                        profile = Path(folder) / f"{name}-{variant}"
                        sample = capture(
                            exes[name, variant], variant, profile, args.idle_seconds, args.timeout, scroll=True,
                        )
                        app_results[variant]["scroll"].append(sample)
                        save()
            for result in app_results.values():
                for phase in ("warm", "scroll"):
                    runs = result[phase]
                    if runs:
                        result[phase + "Median"] = summarized(runs[0], runs)
            save()


if __name__ == "__main__":
    main()
