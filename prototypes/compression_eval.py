#!/usr/bin/env python3
"""Microbenchmark synthetic payloads with system Brotli and Zstandard libraries.

Run with Python 3 on Linux/macOS and shared Brotli encoder/decoder and Zstandard
libraries installed. The script makes no network requests, reads no workspace
files, and adds no Carapana runtime dependency. Timings include the Python
ctypes call boundary and codec API call, but exclude JSON serialization, socket
I/O, TLS, and network transfer. Results are a microbenchmark, not a protocol
integration or production capacity claim.
"""

from __future__ import annotations

import argparse
import ctypes
import ctypes.util
import json
import math
import os
import platform
import resource
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any


@dataclass(frozen=True)
class PayloadCase:
    name: str
    scope: str
    data: bytes


def _deterministic_bytes(size: int, seed: int) -> bytes:
    state = seed & 0xFFFFFFFF
    output = bytearray()
    for _ in range(size):
        state ^= (state << 13) & 0xFFFFFFFF
        state ^= state >> 17
        state ^= (state << 5) & 0xFFFFFFFF
        output.append(state & 0xFF)
    return bytes(output)


def _entropy_json(kind: str, size: int, seed: int) -> bytes:
    prefix = b'{"protocol":1,"type":"' + kind.encode("ascii") + b'","payload":"'
    suffix = b'"}'
    payload_size = size - len(prefix) - len(suffix)
    if payload_size < 0:
        raise ValueError(f"requested JSON payload size is too small: {size}")

    safe_alphabet = (
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"
        b"!#$%&'()*+,-./:;<=>?@[]^_`{|}~"
    )
    random_bytes = _deterministic_bytes(payload_size, seed)
    payload = bytes(safe_alphabet[value % len(safe_alphabet)] for value in random_bytes)
    return prefix + payload + suffix


def _synthetic_record(index: int, record_kind: str) -> dict[str, Any]:
    statuses = ["paused", "running", "waiting_for_user", "waiting_for_approval"]
    details = [
        "Synthetic status transition recorded for benchmark coverage.",
        "Synthetic queue item retained while the session is paused.",
        "Synthetic approval is pending for a local fixture operation.",
        "Synthetic validation completed without external side effects.",
        "Synthetic recovery state is awaiting explicit operator confirmation.",
    ]
    if record_kind == "event":
        return {
            "event_id": f"evt-{index:08d}",
            "sequence": index + 1200,
            "session_id": f"ses-{index % 4096:04x}",
            "type": "session.updated",
            "status": statuses[index % len(statuses)],
            "detail": details[index % len(details)],
        }

    return {
        "id": f"ses-{index:08d}",
        "workspace": f"workspace-{index % 13:02d}",
        "branch": f"feat/recovery-{index % 19:02d}",
        "status": statuses[index % len(statuses)],
        "provider": "chatgpt",
        "model": "gpt-synthetic",
        "updated_at": f"2026-10-19T12:{(index // 60) % 60:02d}:{index % 60:02d}Z",
        "queue_depth": index % 5,
        "summary": f"{details[index % len(details)]} Item {index:05d}.",
    }


def _record_payload(kind: str, size: int, record_kind: str) -> bytes:
    records: list[dict[str, Any]] = []

    def encode() -> bytes:
        document = {"protocol": 1, "type": kind, "records": records, "padding": ""}
        return json.dumps(document, separators=(",", ":"), ensure_ascii=True).encode("ascii")

    while True:
        encoded = encode()
        if len(encoded) >= size:
            if len(encoded) > size and records:
                records.pop()
            break
        records.append(_synthetic_record(len(records), record_kind))

    encoded = encode()
    padding_size = size - len(encoded)
    if padding_size < 0:
        raise ValueError(f"record payload exceeds requested size: {kind}")
    if padding_size == 0:
        return encoded

    marker = b'"padding":""'
    marker_start = encoded.rfind(marker)
    if marker_start < 0:
        raise ValueError("synthetic JSON padding field was not found")
    value_start = marker_start + len(b'"padding":"')
    value_end = marker_start + len(marker) - 1
    pattern = b"synthetic-padding-item-"
    filler = (pattern * ((padding_size + len(pattern) - 1) // len(pattern)))[:padding_size]
    return encoded[:value_start] + filler + encoded[value_end:]


def _diff_payload(size: int) -> bytes:
    parts: list[str] = []
    total_bytes = 0
    while total_bytes < size:
        module = len(parts)
        file_parts = [
            f"diff --git a/src/session_{module:03}.rs b/src/session_{module:03}.rs\n"
            "index 0123456..89abcde 100644\n"
            f"--- a/src/session_{module:03}.rs\n"
            f"+++ b/src/session_{module:03}.rs\n"
            "@@ -1,16 +1,32 @@\n"
        ]
        for index in range(module * 16, (module + 1) * 16):
            file_parts.append(f"-    let attempt_{index:05} = retry_count + {index % 9};\n")
        for index in range(module * 16, (module + 1) * 16):
            file_parts.append(f"+    let attempt_{index:05} = retry_count + {index % 11};\n")
            file_parts.append(
                f"+    let event_{index:05} = SessionEvent::new(\"session-{index % 4096:04x}\");\n"
            )
        part = "".join(file_parts)
        parts.append(part)
        total_bytes += len(part.encode("ascii"))
    return "".join(parts).encode("ascii")


def _sse_payload(size: int) -> bytes:
    parts: list[bytes] = []
    total_bytes = 0
    index = 0
    while total_bytes < size:
        event = {
            "type": "response.output_text.delta",
            "sequence_number": index,
            "delta": (
                f"Synthetic response segment {index:06d} reports a local fixture "
                f"state transition in workspace-{index % 13:02d}."
            ),
        }
        data = json.dumps(event, separators=(",", ":"), ensure_ascii=True).encode("ascii")
        frame = b"event: response.output_text.delta\ndata: " + data + b"\n\n"
        parts.append(frame)
        total_bytes += len(frame)
        index += 1
    parts.append(
        b'event: response.completed\ndata: {"type":"response.completed"}\n\n'
    )
    return b"".join(parts)


def payload_cases() -> list[PayloadCase]:
    cases = [
        PayloadCase(
            "event-256B",
            "local-ipc",
            _record_payload("session.event", 256, "event"),
        ),
        PayloadCase(
            "envelope-4KiB",
            "local-ipc-and-remote-request",
            _record_payload("daemon.envelope", 4 * 1024, "event"),
        ),
        PayloadCase(
            "snapshot-32KiB",
            "local-ipc-and-remote-request",
            _record_payload("session.snapshot", 32 * 1024, "session"),
        ),
        PayloadCase("diff-32KiB", "local-ipc-and-remote-request", _diff_payload(32 * 1024)),
        PayloadCase(
            "frame-64KiB",
            "local-ipc-and-remote-request",
            _record_payload("ipc.frame", 64 * 1024, "session"),
        ),
        PayloadCase(
            "entropy-256B",
            "local-ipc",
            _entropy_json("opaque.payload", 256, 0x185),
        ),
        PayloadCase("opaque-binary-32B", "local-ipc", _deterministic_bytes(32, 0x185)),
        PayloadCase(
            "entropy-64KiB",
            "local-ipc-and-remote-request",
            _entropy_json("opaque.payload", 64 * 1024, 0x185),
        ),
        PayloadCase("sse-stream-256KiB", "remote-sse-model-only", _sse_payload(256 * 1024)),
    ]

    for case in cases:
        if not case.data:
            raise ValueError(f"synthetic case {case.name} is empty")

    return cases


def _shared_library(name: str) -> ctypes.CDLL:
    library = ctypes.util.find_library(name)
    if library is None:
        raise RuntimeError(f"required shared library was not found: {name}")
    return ctypes.CDLL(library)


def _peak_rss_bytes() -> int:
    value = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    if sys.platform == "darwin":
        return int(value)
    if sys.platform.startswith("linux"):
        return int(value * 1024)
    raise RuntimeError(f"peak RSS units are not defined for platform: {sys.platform}")


def _brotli_codec(payload: bytes, level: int) -> tuple[int, Any, Any]:
    encoder = _shared_library("brotlienc")
    decoder = _shared_library("brotlidec")
    encoder.BrotliEncoderMaxCompressedSize.argtypes = [ctypes.c_size_t]
    encoder.BrotliEncoderMaxCompressedSize.restype = ctypes.c_size_t
    encoder.BrotliEncoderCompress.argtypes = [
        ctypes.c_int,
        ctypes.c_int,
        ctypes.c_int,
        ctypes.c_size_t,
        ctypes.c_void_p,
        ctypes.POINTER(ctypes.c_size_t),
        ctypes.c_void_p,
    ]
    encoder.BrotliEncoderCompress.restype = ctypes.c_int
    decoder.BrotliDecoderDecompress.argtypes = [
        ctypes.c_size_t,
        ctypes.c_void_p,
        ctypes.POINTER(ctypes.c_size_t),
        ctypes.c_void_p,
    ]
    decoder.BrotliDecoderDecompress.restype = ctypes.c_int

    source = ctypes.create_string_buffer(payload, len(payload))
    source_pointer = ctypes.cast(source, ctypes.c_void_p)
    capacity = int(encoder.BrotliEncoderMaxCompressedSize(len(payload)))
    if capacity <= 0:
        raise RuntimeError("Brotli returned an invalid output capacity")
    output = ctypes.create_string_buffer(capacity)
    output_pointer = ctypes.cast(output, ctypes.c_void_p)

    def compress() -> int:
        output_size = ctypes.c_size_t(capacity)
        succeeded = encoder.BrotliEncoderCompress(
            level,
            22,
            0,
            len(payload),
            source_pointer,
            ctypes.byref(output_size),
            output_pointer,
        )
        if succeeded != 1:
            raise RuntimeError("Brotli compression failed")
        return int(output_size.value)

    def prepare_decompress(encoded_size: int) -> tuple[Any, Any]:
        encoded = ctypes.create_string_buffer(output.raw[:encoded_size], encoded_size)
        encoded_pointer = ctypes.cast(encoded, ctypes.c_void_p)
        decoded = ctypes.create_string_buffer(len(payload))
        decoded_pointer = ctypes.cast(decoded, ctypes.c_void_p)

        def decompress() -> int:
            decoded_size = ctypes.c_size_t(len(payload))
            succeeded = decoder.BrotliDecoderDecompress(
                encoded_size,
                encoded_pointer,
                ctypes.byref(decoded_size),
                decoded_pointer,
            )
            if succeeded != 1:
                raise RuntimeError("Brotli decompression failed")
            return int(decoded_size.value)

        def verify() -> bool:
            return decoded.raw[: len(payload)] == payload

        return decompress, verify

    return capacity, output, (compress, prepare_decompress)


def _zstd_codec(payload: bytes, level: int) -> tuple[int, Any, Any]:
    library = _shared_library("zstd")
    library.ZSTD_compressBound.argtypes = [ctypes.c_size_t]
    library.ZSTD_compressBound.restype = ctypes.c_size_t
    library.ZSTD_compress.argtypes = [
        ctypes.c_void_p,
        ctypes.c_size_t,
        ctypes.c_void_p,
        ctypes.c_size_t,
        ctypes.c_int,
    ]
    library.ZSTD_compress.restype = ctypes.c_size_t
    library.ZSTD_decompress.argtypes = [
        ctypes.c_void_p,
        ctypes.c_size_t,
        ctypes.c_void_p,
        ctypes.c_size_t,
    ]
    library.ZSTD_decompress.restype = ctypes.c_size_t
    library.ZSTD_isError.argtypes = [ctypes.c_size_t]
    library.ZSTD_isError.restype = ctypes.c_uint
    library.ZSTD_getErrorName.argtypes = [ctypes.c_size_t]
    library.ZSTD_getErrorName.restype = ctypes.c_char_p

    source = ctypes.create_string_buffer(payload, len(payload))
    source_pointer = ctypes.cast(source, ctypes.c_void_p)
    capacity = int(library.ZSTD_compressBound(len(payload)))
    if capacity <= 0:
        raise RuntimeError("Zstandard returned an invalid output capacity")
    output = ctypes.create_string_buffer(capacity)
    output_pointer = ctypes.cast(output, ctypes.c_void_p)

    def check_error(result: int, operation: str) -> int:
        if library.ZSTD_isError(result) != 0:
            error_name = library.ZSTD_getErrorName(result).decode("ascii", "replace")
            raise RuntimeError(f"Zstandard {operation} failed: {error_name}")
        return int(result)

    def compress() -> int:
        result = library.ZSTD_compress(
            output_pointer,
            capacity,
            source_pointer,
            len(payload),
            level,
        )
        return check_error(result, "compression")

    def prepare_decompress(encoded_size: int) -> tuple[Any, Any]:
        encoded = ctypes.create_string_buffer(output.raw[:encoded_size], encoded_size)
        encoded_pointer = ctypes.cast(encoded, ctypes.c_void_p)
        decoded = ctypes.create_string_buffer(len(payload))
        decoded_pointer = ctypes.cast(decoded, ctypes.c_void_p)

        def decompress() -> int:
            result = library.ZSTD_decompress(
                decoded_pointer,
                len(payload),
                encoded_pointer,
                encoded_size,
            )
            return check_error(result, "decompression")

        def verify() -> bool:
            return decoded.raw[: len(payload)] == payload

        return decompress, verify

    return capacity, output, (compress, prepare_decompress)


def _percentile(samples: list[int], percentile: float) -> int:
    ordered = sorted(samples)
    index = max(0, math.ceil(percentile * len(ordered)) - 1)
    return int(ordered[index])


def _worker(codec: str, level: int, iterations: int, payload: bytes) -> dict[str, Any]:
    if codec == "brotli":
        _, output, operations = _brotli_codec(payload, level)
    elif codec == "zstd":
        _, output, operations = _zstd_codec(payload, level)
    else:
        raise ValueError(f"unsupported codec: {codec}")

    compress, prepare_decompress = operations
    warmup_iterations = max(20, iterations // 10)
    compressed_size = compress()
    encoded = output.raw[:compressed_size]
    decompress, verify_decompression = prepare_decompress(compressed_size)
    for _ in range(warmup_iterations):
        if compress() != compressed_size:
            raise RuntimeError("compressed size changed during warmup")
        if decompress() != len(payload) or not verify_decompression():
            raise RuntimeError("decompressed size did not match synthetic input")

    compression_samples: list[int] = []
    compression_cpu_start = time.process_time_ns()
    for _ in range(iterations):
        start = time.perf_counter_ns()
        result_size = compress()
        elapsed = time.perf_counter_ns() - start
        if result_size != compressed_size:
            raise RuntimeError("compressed size changed during repeated samples")
        compression_samples.append(elapsed)
    compression_cpu_ns = time.process_time_ns() - compression_cpu_start

    decompression_samples: list[int] = []
    decompression_cpu_start = time.process_time_ns()
    for _ in range(iterations):
        start = time.perf_counter_ns()
        result_size = decompress()
        elapsed = time.perf_counter_ns() - start
        if result_size != len(payload):
            raise RuntimeError("decompressed size changed during repeated samples")
        decompression_samples.append(elapsed)
    decompression_cpu_ns = time.process_time_ns() - decompression_cpu_start

    peak_rss = _peak_rss_bytes()
    return {
        "codec": codec,
        "level": level,
        "compressed_bytes": len(encoded),
        "ratio": len(encoded) / len(payload),
        "saved_percent": (1.0 - (len(encoded) / len(payload))) * 100.0,
        "iterations": iterations,
        "warmup_iterations": warmup_iterations,
        "compression_p50_ns": _percentile(compression_samples, 0.50),
        "compression_p95_ns": _percentile(compression_samples, 0.95),
        "compression_cpu_ns_per_op": compression_cpu_ns // iterations,
        "decompression_p50_ns": _percentile(decompression_samples, 0.50),
        "decompression_p95_ns": _percentile(decompression_samples, 0.95),
        "decompression_cpu_ns_per_op": decompression_cpu_ns // iterations,
        "process_peak_rss_bytes": peak_rss,
    }


def _command_version(command: str, arguments: list[str]) -> str | None:
    try:
        result = subprocess.run(
            [command, *arguments],
            check=False,
            capture_output=True,
            text=True,
            timeout=5,
        )
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return None

    output = (result.stdout or result.stderr).strip()
    if result.returncode != 0 or not output:
        return None
    return output.splitlines()[0]


def _cpu_model() -> str | None:
    if sys.platform.startswith("linux"):
        try:
            for line in Path("/proc/cpuinfo").read_text(encoding="utf-8").splitlines():
                if line.startswith("model name"):
                    _, value = line.split(":", maxsplit=1)
                    return value.strip()
        except OSError:
            return None
    return platform.processor() or None


def _baseline(case: PayloadCase) -> dict[str, Any]:
    return {
        "case": case.name,
        "scope": case.scope,
        "raw_bytes": len(case.data),
        "codec": "none",
        "compressed_bytes": len(case.data),
        "ratio": 1.0,
        "saved_percent": 0.0,
        "compression_p50_ns": 0,
        "compression_p95_ns": 0,
        "compression_cpu_ns_per_op": 0,
        "decompression_p50_ns": 0,
        "decompression_p95_ns": 0,
        "decompression_cpu_ns_per_op": 0,
        "measurement": "no codec call; serialization and I/O excluded",
    }


def _run_case(case: PayloadCase, codec: str, level: int, iterations: int) -> dict[str, Any]:
    command = [
        sys.executable,
        str(Path(__file__).resolve()),
        "--worker",
        codec,
        str(level),
        str(iterations),
    ]
    completed = subprocess.run(
        command,
        input=case.data,
        capture_output=True,
        check=False,
        timeout=120,
    )
    if completed.returncode != 0:
        message = completed.stderr.decode("utf-8", "replace").strip()
        raise RuntimeError(f"benchmark worker failed for {case.name}/{codec}-{level}: {message}")

    measurement = json.loads(completed.stdout)
    measurement["case"] = case.name
    measurement["scope"] = case.scope
    measurement["raw_bytes"] = len(case.data)
    measurement["measurement"] = "Python ctypes + native one-shot codec API"
    return measurement


def run_benchmark(iterations: int) -> dict[str, Any]:
    if iterations < 50:
        raise ValueError("use at least 50 iterations for a meaningful p95 sample")

    cases = payload_cases()
    measurements: list[dict[str, Any]] = []
    for case in cases:
        measurements.append(_baseline(case))
        for codec, levels in [("brotli", [4, 6]), ("zstd", [1, 3])]:
            for level in levels:
                measurements.append(_run_case(case, codec, level, iterations))

    return {
        "schema_version": 1,
        "environment": {
            "system": platform.system(),
            "release": platform.release(),
            "machine": platform.machine(),
            "processor": platform.processor() or None,
            "cpu_model": _cpu_model(),
            "logical_cpus": os.cpu_count(),
            "python": platform.python_version(),
            "brotli_cli": _command_version("brotli", ["--version"]),
            "zstd_cli": _command_version("zstd", ["--version"]),
            "iterations_per_phase": iterations,
            "warmup_iterations_per_worker": max(20, iterations // 10),
            "timing_note": "ctypes/Python wrapper included; serialization, socket, TLS, and network excluded",
            "memory_note": "absolute peak RSS includes the Python worker and native codec libraries; it is not codec-only allocation",
        },
        "measurements": measurements,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--iterations", type=int, default=300)
    parser.add_argument("--worker", nargs=3, metavar=("CODEC", "LEVEL", "ITERATIONS"))
    arguments = parser.parse_args()

    if arguments.worker is not None:
        codec, raw_level, raw_iterations = arguments.worker
        payload = sys.stdin.buffer.read()
        measurement = _worker(codec, int(raw_level), int(raw_iterations), payload)
        sys.stdout.write(json.dumps(measurement, separators=(",", ":")))
        return 0

    report = run_benchmark(arguments.iterations)
    sys.stdout.write(json.dumps(report, indent=2, sort_keys=True) + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
