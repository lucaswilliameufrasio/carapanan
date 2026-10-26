#!/usr/bin/env python3
"""Measure bounded local daemon session and durable-queue workloads on macOS."""

from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import asdict, dataclass
import hashlib
from math import ceil
import json
import os
from pathlib import Path
import platform
import signal
from statistics import median
import subprocess
import tempfile
import threading
import time


CLI = Path(__file__).parents[1] / "target/release/carapanan"
SAMPLE_INTERVAL_SECONDS = 0.1
BASELINE_SETTLE_SECONDS = 1
COMMAND_TIMEOUT_SECONDS = 15
DAEMON_READY_TIMEOUT_SECONDS = 10
WORKLOADS = (
    {
        "name": "typical-queued-work",
        "sessions": 4,
        "messages_per_session": 8,
        "message_bytes": 1024,
        "producer_concurrency": 1,
    },
    {
        "name": "bounded-queue-burst",
        "sessions": 8,
        "messages_per_session": 4,
        "message_bytes": 8192,
        "producer_concurrency": 4,
    },
)


@dataclass(frozen=True)
class Workload:
    name: str
    sessions: int
    messages_per_session: int
    message_bytes: int
    producer_concurrency: int

    @property
    def request_count(self):
        return self.sessions * self.messages_per_session

    @property
    def payload_bytes(self):
        return self.request_count * self.message_bytes


class ProcessRegistry:
    def __init__(self, daemon_process_id):
        self.daemon_process_id = daemon_process_id
        self.client_process_ids = set()
        self.lock = threading.Lock()

    def add_client(self, process_id):
        with self.lock:
            self.client_process_ids.add(process_id)

    def remove_client(self, process_id):
        with self.lock:
            self.client_process_ids.discard(process_id)

    def process_ids(self):
        with self.lock:
            return [self.daemon_process_id, *self.client_process_ids]


class DaemonSampler:
    def __init__(self, process_registry):
        self.process_registry = process_registry
        self.samples_kib = []
        self.daemon_samples_kib = []
        self.stop_event = threading.Event()
        self.thread = threading.Thread(target=self._sample, daemon=True)

    def start(self):
        self.thread.start()

    def stop(self):
        self.stop_event.set()
        self.thread.join(timeout=COMMAND_TIMEOUT_SECONDS + 1)
        if self.thread.is_alive():
            raise TimeoutError("daemon RSS sampler did not stop")

    def _sample(self):
        while not self.stop_event.is_set():
            process_ids = self.process_registry.process_ids()
            result = subprocess.run(
                ["/bin/ps", "-o", "pid=,rss=", "-p", ",".join(map(str, process_ids))],
                check=False,
                capture_output=True,
                text=True,
                timeout=COMMAND_TIMEOUT_SECONDS,
            )
            if result.returncode == 0:
                rss_by_process = {}
                for line in result.stdout.splitlines():
                    columns = line.split()
                    if len(columns) == 2 and columns[0].isdigit() and columns[1].isdigit():
                        rss_by_process[int(columns[0])] = int(columns[1])
                daemon_rss = rss_by_process.get(self.process_registry.daemon_process_id)
                if daemon_rss is not None:
                    self.daemon_samples_kib.append(daemon_rss)
                    self.samples_kib.append(sum(rss_by_process.values()))
            self.stop_event.wait(SAMPLE_INTERVAL_SECONDS)


def run_cli(
    arguments,
    environment,
    input_text=None,
    timeout=COMMAND_TIMEOUT_SECONDS,
    process_registry=None,
):
    command = [str(CLI), *arguments]
    process = subprocess.Popen(
        command,
        stdin=subprocess.PIPE if input_text is not None else subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        env=environment,
    )
    if process_registry is not None:
        process_registry.add_client(process.pid)
    try:
        stdout, stderr = process.communicate(input_text, timeout=timeout)
    except subprocess.TimeoutExpired:
        process.kill()
        process.communicate()
        raise
    finally:
        if process_registry is not None:
            process_registry.remove_client(process.pid)
    return subprocess.CompletedProcess(command, process.returncode, stdout, stderr)


def run_checked_cli(arguments, environment, input_text=None, process_registry=None):
    result = run_cli(arguments, environment, input_text, process_registry=process_registry)
    if result.returncode != 0:
        raise RuntimeError(f"Carapana CLI request failed: {arguments[0]}")
    return result.stdout


def wait_until_ready(environment, deadline):
    while time.monotonic() < deadline:
        try:
            result = run_cli(["sessions", "--json"], environment, timeout=2)
            if result.returncode == 0:
                return
        except subprocess.TimeoutExpired:
            # A cold database initialization can outlast one short readiness attempt.
            pass
        time.sleep(0.05)
    raise TimeoutError("daemon did not become ready")


def create_sessions(workload, workspace, environment):
    session_ids = []
    for _ in range(workload.sessions):
        output = run_checked_cli(["create", "--workspace", str(workspace)], environment)
        prefix = "Created paused session "
        suffix = ". No provider or tools were started.\n"
        session_id = output.removeprefix(prefix).removesuffix(suffix)
        if not session_id or session_id == output:
            raise RuntimeError("session creation returned an invalid response")
        session_ids.append(session_id)
    return session_ids


def queue_message(session_id, text, environment, process_registry):
    started = time.monotonic()
    payload = json.dumps(
        {
            "text": text,
            "selection": {
                "profile": "ask",
                "work": "plan",
                "autonomy": "ask",
                "provider": "synthetic",
                "model": "synthetic",
                "variant": "default",
            },
        },
        separators=(",", ":"),
    )
    run_checked_cli(["queue", session_id], environment, payload, process_registry)
    return (time.monotonic() - started) * 1000


def directory_size(directory):
    total_bytes = 0
    for path in Path(directory).rglob("*"):
        if path.is_file():
            total_bytes += path.stat().st_size
    return total_bytes


def percentile(values, fraction):
    ordered = sorted(values)
    index = max(0, min(len(ordered) - 1, ceil(len(ordered) * fraction) - 1))
    return ordered[index]


def measure_once(workload, repetition):
    with tempfile.TemporaryDirectory(prefix="cr-session-bench-", dir="/private/tmp") as temporary_root:
        root = Path(temporary_root)
        home = root / "home"
        workspace = root / "workspace"
        home.mkdir(mode=0o700)
        workspace.mkdir(mode=0o700)
        environment = {"HOME": str(home), "PATH": os.defpath, "TMPDIR": str(root), "LANG": "C"}
        daemon = subprocess.Popen(
            [str(CLI), "daemon"],
            cwd=root,
            env=environment,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
        sampler = None
        try:
            wait_until_ready(environment, time.monotonic() + DAEMON_READY_TIMEOUT_SECONDS)
            session_ids = create_sessions(workload, workspace, environment)
            process_registry = ProcessRegistry(daemon.pid)
            sampler = DaemonSampler(process_registry)
            sampler.start()
            time.sleep(BASELINE_SETTLE_SECONDS)
            baseline_samples = sampler.samples_kib[:]
            baseline_rss_kib = int(median(baseline_samples)) if baseline_samples else 0

            payload = "x" * workload.message_bytes
            requests = [
                (session_ids[index % len(session_ids)], payload)
                for index in range(workload.request_count)
            ]
            started = time.monotonic()
            with ThreadPoolExecutor(max_workers=workload.producer_concurrency) as executor:
                futures = []
                for session_id, message in requests:
                    futures.append(
                        executor.submit(
                            queue_message,
                            session_id,
                            message,
                            environment,
                            process_registry,
                        )
                    )
                latencies_ms = []
                for future in as_completed(futures, timeout=120):
                    latencies_ms.append(future.result())
            elapsed_seconds = time.monotonic() - started
            time.sleep(0.2)
            peak_rss_kib = max(sampler.samples_kib, default=0)
            database_bytes_before_shutdown = directory_size(home)
            for session_id in session_ids:
                response = run_checked_cli(["show", session_id, "--json"], environment)
                session = json.loads(response)["snapshot"]
                queued_messages = session["queued_messages"]
                if len(queued_messages) != workload.messages_per_session:
                    raise RuntimeError("persisted queue count did not match the workload")
        finally:
            try:
                if sampler is not None:
                    sampler.stop()
            finally:
                if daemon.poll() is None:
                    daemon.send_signal(signal.SIGTERM)
                    try:
                        daemon.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        daemon.kill()
                        daemon.wait(timeout=2)
        database_bytes_after_shutdown = directory_size(home)

    return {
        "workload": asdict(workload),
        "repetition": repetition,
        "queue_requests": workload.request_count,
        "synthetic_payload_bytes": workload.payload_bytes,
        "wall_seconds": round(elapsed_seconds, 4),
        "enqueue_latency_ms_p50": round(percentile(latencies_ms, 0.50), 3),
        "enqueue_latency_ms_p95": round(percentile(latencies_ms, 0.95), 3),
        "daemon_plus_cli_rss_baseline_kib_sampled": baseline_rss_kib,
        "daemon_plus_cli_rss_peak_kib_sampled": peak_rss_kib,
        "daemon_rss_peak_kib_sampled": max(sampler.daemon_samples_kib, default=0),
        "storage_bytes_before_shutdown": database_bytes_before_shutdown,
        "storage_bytes_after_shutdown": database_bytes_after_shutdown,
        "rss_sampling_interval_ms": int(SAMPLE_INTERVAL_SECONDS * 1000),
        "scope": "local-daemon-session-and-durable-queue-only",
    }


def main():
    if platform.system() != "Darwin":
        raise SystemExit("This workload runner is intended for macOS.")
    if not CLI.is_file():
        raise SystemExit("Build the release carapanan CLI before running this benchmark.")

    results = []
    warmup = Workload("warmup", 1, 1, 128, 1)
    measure_once(warmup, 0)
    for workload_data in WORKLOADS:
        workload = Workload(**workload_data)
        for repetition in range(1, 4):
            results.append(measure_once(workload, repetition))
    toolchain_result = subprocess.run(
        [
            "bash",
            str(Path(__file__).parents[1] / "scripts/rust-toolchain-command.sh"),
            "rustc",
            "--version",
        ],
        check=True,
        capture_output=True,
        text=True,
    )
    digest = hashlib.sha256(CLI.read_bytes()).hexdigest()
    print(
        json.dumps(
            {
                "schema_version": 1,
                "platform": platform.platform(),
                "rustc_version": toolchain_result.stdout.strip(),
                "cli_binary_sha256": digest,
                "results": results,
            },
            indent=2,
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
