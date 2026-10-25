#!/usr/bin/env python3
"""One bounded sample of the explicitly supplied trusted release probe, not a sandbox."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import selectors
import signal
import subprocess
import sys
import tempfile
import time

MAX_OUTPUT = 8192
TIMEOUT = 30


def validate_probe(value):
    if not isinstance(value, dict):
        raise ValueError("invalid probe result")
    expected = {
        "schema_version", "scope", "iterations", "nominal_bytes_per_intent", "open_us",
        "immediate_total_us", "enqueue_total_us", "claim_total_us",
        "remaining_reserved_bytes", "remaining_waiting", "production_defaults_calibrated",
    }
    if "self_rss_peak_bytes" in value:
        expected.add("self_rss_peak_bytes")
    if set(value) != expected or value["schema_version"] != 1:
        raise ValueError("invalid probe schema")
    if value["scope"] != "sqlite-admission-control-plane-only" or value["production_defaults_calibrated"] is not False:
        raise ValueError("invalid probe scope")
    for key in expected - {"scope", "production_defaults_calibrated"}:
        if type(value[key]) is not int or not 0 <= value[key] <= 2**63 - 1:
            raise ValueError("invalid probe metric")
    if value["iterations"] != 16 or value["nominal_bytes_per_intent"] != 1024 * 1024:
        raise ValueError("unexpected probe workload")
    if value["remaining_reserved_bytes"] or value["remaining_waiting"]:
        raise ValueError("probe retained capacity")
    return value


def measure(binary):
    if sys.platform not in ("linux", "darwin"):
        raise ValueError("unsupported measurement platform")
    binary = Path(binary).resolve(strict=True)
    if not binary.is_file() or binary.stat().st_size > 128 * 1024 * 1024:
        raise ValueError("invalid probe binary")
    digest = hashlib.sha256()
    with binary.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix="carapana-probe-run-") as root:
        process = subprocess.Popen([str(binary)], cwd=root, env={"PATH": os.defpath, "TMPDIR": root},
                                   stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                   stderr=subprocess.DEVNULL, start_new_session=True)
        output = bytearray()
        usage = None
        try:
            with selectors.DefaultSelector() as selector:
                selector.register(process.stdout, selectors.EVENT_READ)
                while selector.get_map() or usage is None:
                    if time.monotonic() - started > TIMEOUT:
                        raise TimeoutError("probe timed out")
                    for key, _ in selector.select(0.02):
                        chunk = os.read(key.fileobj.fileno(), 4096)
                        if not chunk:
                            selector.unregister(key.fileobj)
                        output.extend(chunk)
                        if len(output) > MAX_OUTPUT:
                            raise ValueError("probe output budget exceeded")
                    if usage is None:
                        pid, status, sample = os.wait4(process.pid, os.WNOHANG)
                        if pid:
                            process.returncode = os.waitstatus_to_exitcode(status)
                            usage = sample
                if process.returncode != 0:
                    raise ValueError("probe failed")
        finally:
            if usage is None:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                _, status, _ = os.wait4(process.pid, 0)
                process.returncode = os.waitstatus_to_exitcode(status)
            process.stdout.close()
    return {
        "schema_version": 1,
        "binary_sha256": digest.hexdigest(),
        "platform": sys.platform,
        "wall_ms": round((time.monotonic() - started) * 1000, 3),
        "child_max_rss_bytes": int(usage.ru_maxrss) * (1024 if sys.platform == "linux" else 1),
        "rusage_includes_preexec": True,
        "child_user_cpu_seconds": usage.ru_utime,
        "child_system_cpu_seconds": usage.ru_stime,
        "probe": validate_probe(json.loads(output)),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True)
    arguments = parser.parse_args()
    try:
        print(json.dumps(measure(arguments.binary), sort_keys=True))
    except (OSError, ValueError, TimeoutError):
        # No raw child output, command lines, paths, environment or credentials in diagnostics.
        parser.exit(1, "admission probe measurement failed\n")


if __name__ == "__main__":
    main()
