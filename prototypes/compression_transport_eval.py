#!/usr/bin/env python3
"""Measure incremental codec streams over a local Unix socket pair.

This is an exploratory benchmark only. It uses deterministic synthetic payloads,
system Brotli/Zstandard shared libraries, and a local ``socketpair``; it never
opens workspace files or makes network/provider requests. It does not model the
Carapana IPC framing, HTTP headers, TLS, or a remote network.
"""

from __future__ import annotations

import argparse
import ctypes
import ctypes.util
import json
import math
import platform
import socket
import statistics
import sys
import threading
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

if __package__:
    from .compression_eval import _sse_payload, payload_cases
else:
    from compression_eval import _sse_payload, payload_cases


_OUTPUT_BUFFER_BYTES = 16 * 1024
_BYTE = ctypes.c_uint8
_BYTE_POINTER = ctypes.POINTER(_BYTE)
_BYTE_POINTER_POINTER = ctypes.POINTER(_BYTE_POINTER)


class _ZstdOutBuffer(ctypes.Structure):
    _fields_ = [
        ("dst", ctypes.c_void_p),
        ("size", ctypes.c_size_t),
        ("pos", ctypes.c_size_t),
    ]


class _ZstdInBuffer(ctypes.Structure):
    _fields_ = [
        ("src", ctypes.c_void_p),
        ("size", ctypes.c_size_t),
        ("pos", ctypes.c_size_t),
    ]


def _shared_library(name: str) -> ctypes.CDLL:
    library_name = ctypes.util.find_library(name)
    if library_name is None:
        raise RuntimeError(f"required shared library was not found: {name}")
    return ctypes.CDLL(library_name)


def _input_storage(data: bytes) -> tuple[Any, _BYTE_POINTER]:
    storage = (_BYTE * max(1, len(data)))()
    if data:
        ctypes.memmove(storage, data, len(data))
    return storage, ctypes.cast(storage, _BYTE_POINTER)


def _check_output_limit(current_size: int, added_size: int, maximum: int) -> None:
    if current_size + added_size > maximum:
        raise ValueError("decompressed output exceeds the configured byte limit")


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


def _native_versions() -> dict[str, str]:
    brotli = _shared_library("brotlienc")
    brotli.BrotliEncoderVersion.restype = ctypes.c_uint32
    brotli_raw_version = int(brotli.BrotliEncoderVersion())
    brotli_version = (
        f"{brotli_raw_version >> 24}."
        f"{(brotli_raw_version >> 12) & 0xFFF}."
        f"{brotli_raw_version & 0xFFF}"
    )

    zstd = _shared_library("zstd")
    zstd.ZSTD_versionString.restype = ctypes.c_char_p
    zstd_version = zstd.ZSTD_versionString().decode("ascii", "replace")
    return {"brotli": brotli_version, "zstd": zstd_version}


class _BrotliEncoder:
    def __init__(self, level: int) -> None:
        self._library = _shared_library("brotlienc")
        self._library.BrotliEncoderCreateInstance.argtypes = [
            ctypes.c_void_p,
            ctypes.c_void_p,
            ctypes.c_void_p,
        ]
        self._library.BrotliEncoderCreateInstance.restype = ctypes.c_void_p
        self._library.BrotliEncoderDestroyInstance.argtypes = [ctypes.c_void_p]
        self._library.BrotliEncoderSetParameter.argtypes = [
            ctypes.c_void_p,
            ctypes.c_int,
            ctypes.c_uint32,
        ]
        self._library.BrotliEncoderSetParameter.restype = ctypes.c_int
        self._library.BrotliEncoderCompressStream.argtypes = [
            ctypes.c_void_p,
            ctypes.c_int,
            ctypes.POINTER(ctypes.c_size_t),
            _BYTE_POINTER_POINTER,
            ctypes.POINTER(ctypes.c_size_t),
            _BYTE_POINTER_POINTER,
            ctypes.POINTER(ctypes.c_size_t),
        ]
        self._library.BrotliEncoderCompressStream.restype = ctypes.c_int
        self._library.BrotliEncoderHasMoreOutput.argtypes = [ctypes.c_void_p]
        self._library.BrotliEncoderHasMoreOutput.restype = ctypes.c_int
        self._library.BrotliEncoderIsFinished.argtypes = [ctypes.c_void_p]
        self._library.BrotliEncoderIsFinished.restype = ctypes.c_int

        self._state = self._library.BrotliEncoderCreateInstance(None, None, None)
        if not self._state:
            raise RuntimeError("Brotli encoder allocation failed")
        if self._library.BrotliEncoderSetParameter(self._state, 1, level) != 1:
            self.close()
            raise RuntimeError("Brotli encoder rejected the requested quality")

    def _run(self, data: bytes, operation: int) -> bytes:
        input_storage, next_input = _input_storage(data)
        available_input = ctypes.c_size_t(len(data))
        output = bytearray()
        while True:
            output_storage = ctypes.create_string_buffer(_OUTPUT_BUFFER_BYTES)
            next_output = ctypes.cast(output_storage, _BYTE_POINTER)
            available_output = ctypes.c_size_t(_OUTPUT_BUFFER_BYTES)
            total_output = ctypes.c_size_t()
            previous_input = available_input.value

            succeeded = self._library.BrotliEncoderCompressStream(
                self._state,
                operation,
                ctypes.byref(available_input),
                ctypes.byref(next_input),
                ctypes.byref(available_output),
                ctypes.byref(next_output),
                ctypes.byref(total_output),
            )
            if succeeded != 1:
                raise RuntimeError("Brotli streaming compression failed")

            produced = _OUTPUT_BUFFER_BYTES - available_output.value
            if produced:
                output.extend(ctypes.string_at(output_storage, produced))

            if operation == 2 and self._library.BrotliEncoderIsFinished(self._state):
                break
            if (
                operation != 2
                and available_input.value == 0
                and not self._library.BrotliEncoderHasMoreOutput(self._state)
            ):
                break
            if available_input.value == previous_input and produced == 0:
                raise RuntimeError("Brotli encoder made no progress")

        # Keep the input allocation alive until all native calls have returned.
        del input_storage
        return bytes(output)

    def write(self, data: bytes) -> bytes:
        return self._run(data, 1)

    def finish(self) -> bytes:
        return self._run(b"", 2)

    def close(self) -> None:
        state = getattr(self, "_state", None)
        if state:
            self._library.BrotliEncoderDestroyInstance(state)
            self._state = None

    def __del__(self) -> None:
        self.close()


class _BrotliDecoder:
    def __init__(self, maximum_output_bytes: int) -> None:
        self._library = _shared_library("brotlidec")
        self._library.BrotliDecoderCreateInstance.argtypes = [
            ctypes.c_void_p,
            ctypes.c_void_p,
            ctypes.c_void_p,
        ]
        self._library.BrotliDecoderCreateInstance.restype = ctypes.c_void_p
        self._library.BrotliDecoderDestroyInstance.argtypes = [ctypes.c_void_p]
        self._library.BrotliDecoderDecompressStream.argtypes = [
            ctypes.c_void_p,
            ctypes.POINTER(ctypes.c_size_t),
            _BYTE_POINTER_POINTER,
            ctypes.POINTER(ctypes.c_size_t),
            _BYTE_POINTER_POINTER,
            ctypes.POINTER(ctypes.c_size_t),
        ]
        self._library.BrotliDecoderDecompressStream.restype = ctypes.c_int
        self._library.BrotliDecoderIsFinished.argtypes = [ctypes.c_void_p]
        self._library.BrotliDecoderIsFinished.restype = ctypes.c_int

        self._state = self._library.BrotliDecoderCreateInstance(None, None, None)
        if not self._state:
            raise RuntimeError("Brotli decoder allocation failed")
        self._maximum_output_bytes = maximum_output_bytes
        self._total_output_bytes = 0
        self._finished = False

    def feed(self, data: bytes) -> bytes:
        if self._finished:
            if data:
                raise ValueError("trailing bytes after the Brotli stream")
            return b""

        input_storage, next_input = _input_storage(data)
        available_input = ctypes.c_size_t(len(data))
        output = bytearray()
        while True:
            output_storage = ctypes.create_string_buffer(_OUTPUT_BUFFER_BYTES)
            next_output = ctypes.cast(output_storage, _BYTE_POINTER)
            available_output = ctypes.c_size_t(_OUTPUT_BUFFER_BYTES)
            total_output = ctypes.c_size_t()
            previous_input = available_input.value
            result = self._library.BrotliDecoderDecompressStream(
                self._state,
                ctypes.byref(available_input),
                ctypes.byref(next_input),
                ctypes.byref(available_output),
                ctypes.byref(next_output),
                ctypes.byref(total_output),
            )
            produced = _OUTPUT_BUFFER_BYTES - available_output.value
            _check_output_limit(
                self._total_output_bytes + len(output),
                produced,
                self._maximum_output_bytes,
            )
            if produced:
                output.extend(ctypes.string_at(output_storage, produced))

            if result == 0:
                raise ValueError("invalid Brotli stream")
            if result == 1:
                if available_input.value:
                    raise ValueError("trailing bytes after the Brotli stream")
                self._finished = True
                break
            if result == 2:
                if available_input.value == previous_input and produced == 0:
                    raise RuntimeError("Brotli decoder made no progress")
                if available_input.value:
                    continue
                break
            if result != 3:
                raise RuntimeError(f"unexpected Brotli decoder status: {result}")
            if available_input.value == previous_input and produced == 0:
                raise RuntimeError("Brotli decoder made no progress")

        self._total_output_bytes += len(output)
        del input_storage
        return bytes(output)

    def finish(self) -> None:
        if not self._finished or not self._library.BrotliDecoderIsFinished(self._state):
            raise ValueError("truncated Brotli stream")

    def close(self) -> None:
        state = getattr(self, "_state", None)
        if state:
            self._library.BrotliDecoderDestroyInstance(state)
            self._state = None

    def __del__(self) -> None:
        self.close()


class _ZstdCodec:
    def __init__(self) -> None:
        self._library = _shared_library("zstd")
        self._library.ZSTD_isError.argtypes = [ctypes.c_size_t]
        self._library.ZSTD_isError.restype = ctypes.c_uint
        self._library.ZSTD_getErrorName.argtypes = [ctypes.c_size_t]
        self._library.ZSTD_getErrorName.restype = ctypes.c_char_p

    def check_result(self, result: int, operation: str) -> int:
        if self._library.ZSTD_isError(result):
            message = self._library.ZSTD_getErrorName(result).decode("ascii", "replace")
            raise ValueError(f"Zstandard {operation} failed: {message}")
        return int(result)


class _ZstdEncoder(_ZstdCodec):
    def __init__(self, level: int) -> None:
        super().__init__()
        self._library.ZSTD_createCStream.restype = ctypes.c_void_p
        self._library.ZSTD_freeCStream.argtypes = [ctypes.c_void_p]
        self._library.ZSTD_initCStream.argtypes = [ctypes.c_void_p, ctypes.c_int]
        self._library.ZSTD_initCStream.restype = ctypes.c_size_t
        self._library.ZSTD_compressStream2.argtypes = [
            ctypes.c_void_p,
            ctypes.POINTER(_ZstdOutBuffer),
            ctypes.POINTER(_ZstdInBuffer),
            ctypes.c_int,
        ]
        self._library.ZSTD_compressStream2.restype = ctypes.c_size_t

        self._stream = self._library.ZSTD_createCStream()
        if not self._stream:
            raise RuntimeError("Zstandard encoder allocation failed")
        self.check_result(self._library.ZSTD_initCStream(self._stream, level), "initialization")

    def _run(self, data: bytes, directive: int) -> bytes:
        input_storage, input_pointer = _input_storage(data)
        input_buffer = _ZstdInBuffer(
            ctypes.cast(input_pointer, ctypes.c_void_p),
            len(data),
            0,
        )
        output = bytearray()

        while True:
            output_storage = ctypes.create_string_buffer(_OUTPUT_BUFFER_BYTES)
            output_buffer = _ZstdOutBuffer(
                ctypes.cast(output_storage, ctypes.c_void_p),
                _OUTPUT_BUFFER_BYTES,
                0,
            )
            previous_position = input_buffer.pos
            remaining = self.check_result(
                self._library.ZSTD_compressStream2(
                    self._stream,
                    ctypes.byref(output_buffer),
                    ctypes.byref(input_buffer),
                    directive,
                ),
                "streaming compression",
            )
            if output_buffer.pos:
                output.extend(ctypes.string_at(output_storage, output_buffer.pos))

            if directive == 2 and remaining == 0:
                break
            if (
                directive == 1
                and input_buffer.pos == input_buffer.size
                and remaining == 0
            ):
                break
            if input_buffer.pos == previous_position and output_buffer.pos == 0:
                raise RuntimeError("Zstandard encoder made no progress")

        del input_storage
        return bytes(output)

    def write(self, data: bytes) -> bytes:
        return self._run(data, 1)

    def finish(self) -> bytes:
        return self._run(b"", 2)

    def close(self) -> None:
        stream = getattr(self, "_stream", None)
        if stream:
            self._library.ZSTD_freeCStream(stream)
            self._stream = None

    def __del__(self) -> None:
        self.close()


class _ZstdDecoder(_ZstdCodec):
    def __init__(self, maximum_output_bytes: int) -> None:
        super().__init__()
        self._library.ZSTD_createDStream.restype = ctypes.c_void_p
        self._library.ZSTD_freeDStream.argtypes = [ctypes.c_void_p]
        self._library.ZSTD_initDStream.argtypes = [ctypes.c_void_p]
        self._library.ZSTD_initDStream.restype = ctypes.c_size_t
        self._library.ZSTD_decompressStream.argtypes = [
            ctypes.c_void_p,
            ctypes.POINTER(_ZstdOutBuffer),
            ctypes.POINTER(_ZstdInBuffer),
        ]
        self._library.ZSTD_decompressStream.restype = ctypes.c_size_t

        self._stream = self._library.ZSTD_createDStream()
        if not self._stream:
            raise RuntimeError("Zstandard decoder allocation failed")
        self.check_result(self._library.ZSTD_initDStream(self._stream), "initialization")
        self._maximum_output_bytes = maximum_output_bytes
        self._total_output_bytes = 0
        self._last_hint: int | None = None

    def feed(self, data: bytes) -> bytes:
        input_storage, input_pointer = _input_storage(data)
        input_buffer = _ZstdInBuffer(
            ctypes.cast(input_pointer, ctypes.c_void_p),
            len(data),
            0,
        )
        output = bytearray()

        need_more_output = False
        while input_buffer.pos < input_buffer.size or need_more_output:
            output_storage = ctypes.create_string_buffer(_OUTPUT_BUFFER_BYTES)
            output_buffer = _ZstdOutBuffer(
                ctypes.cast(output_storage, ctypes.c_void_p),
                _OUTPUT_BUFFER_BYTES,
                0,
            )
            previous_position = input_buffer.pos
            self._last_hint = self.check_result(
                self._library.ZSTD_decompressStream(
                    self._stream,
                    ctypes.byref(output_buffer),
                    ctypes.byref(input_buffer),
                ),
                "streaming decompression",
            )
            _check_output_limit(
                self._total_output_bytes + len(output),
                output_buffer.pos,
                self._maximum_output_bytes,
            )
            if output_buffer.pos:
                output.extend(ctypes.string_at(output_storage, output_buffer.pos))
            need_more_output = (
                output_buffer.pos == _OUTPUT_BUFFER_BYTES and self._last_hint != 0
            )
            if input_buffer.pos == previous_position and output_buffer.pos == 0:
                if input_buffer.pos < input_buffer.size:
                    raise RuntimeError("Zstandard decoder made no progress")
                need_more_output = False

        self._total_output_bytes += len(output)
        del input_storage
        return bytes(output)

    def finish(self) -> None:
        if self._last_hint != 0:
            raise ValueError("truncated Zstandard stream")

    def close(self) -> None:
        stream = getattr(self, "_stream", None)
        if stream:
            self._library.ZSTD_freeDStream(stream)
            self._stream = None

    def __del__(self) -> None:
        self.close()


@dataclass(frozen=True)
class _Profile:
    name: str
    level: int


def _encoder(profile: _Profile) -> _BrotliEncoder | _ZstdEncoder | None:
    if profile.name == "none":
        return None
    if profile.name == "brotli":
        return _BrotliEncoder(profile.level)
    if profile.name == "zstd":
        return _ZstdEncoder(profile.level)
    raise ValueError(f"unsupported profile: {profile.name}")


def _decoder(
    profile: _Profile,
    maximum_output_bytes: int,
) -> _BrotliDecoder | _ZstdDecoder | None:
    if profile.name == "none":
        return None
    if profile.name == "brotli":
        return _BrotliDecoder(maximum_output_bytes)
    if profile.name == "zstd":
        return _ZstdDecoder(maximum_output_bytes)
    raise ValueError(f"unsupported profile: {profile.name}")


def _percentile(samples: list[int], percentile: float) -> int:
    ordered = sorted(samples)
    index = max(0, math.ceil(percentile * len(ordered)) - 1)
    return int(ordered[index])


def _transport_once(
    case_name: str,
    chunks: list[bytes],
    profile: _Profile,
) -> dict[str, Any]:
    expected = b"".join(chunks)
    is_sse = case_name.startswith("sse-")
    encoder = _encoder(profile)
    decoder = _decoder(profile, len(expected))
    receiver, sender = socket.socketpair(socket.AF_UNIX, socket.SOCK_STREAM)
    receiver.settimeout(10)
    sender_errors: list[Exception] = []
    sender_metrics: dict[str, int] = {}

    def send_stream() -> None:
        cpu_started = time.thread_time_ns()
        try:
            for chunk in chunks:
                encoded = encoder.write(chunk) if encoder is not None else chunk
                if encoded:
                    sender.sendall(encoded)
            if encoder is not None:
                tail = encoder.finish()
                if tail:
                    sender.sendall(tail)
            sender_metrics["cpu_ns"] = time.thread_time_ns() - cpu_started
            sender.shutdown(socket.SHUT_WR)
        except Exception as error:
            sender_errors.append(error)
            try:
                sender.shutdown(socket.SHUT_WR)
            except OSError:
                pass

    sender_thread = threading.Thread(target=send_stream, name="compression-socket-sender")
    decoded = bytearray()
    wire_bytes = 0
    first_event_ns: int | None = None
    started = time.perf_counter_ns()
    receiver_cpu_started = time.thread_time_ns()
    sender_thread.start()

    try:
        while True:
            wire_chunk = receiver.recv(_OUTPUT_BUFFER_BYTES)
            if not wire_chunk:
                break
            wire_bytes += len(wire_chunk)
            decoded_chunk = decoder.feed(wire_chunk) if decoder is not None else wire_chunk
            if decoded_chunk:
                decoded.extend(decoded_chunk)
                if is_sse and first_event_ns is None and b"\n\n" in decoded:
                    first_event_ns = time.perf_counter_ns() - started

        if decoder is not None:
            decoder.finish()
        receiver_cpu_ns = time.thread_time_ns() - receiver_cpu_started
    finally:
        receiver.close()
        sender_thread.join(timeout=10)
        if sender_thread.is_alive():
            try:
                sender.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            sender_thread.join(timeout=1)
        if decoder is not None:
            decoder.close()
        if sender_thread.is_alive():
            raise TimeoutError("local socket sender did not stop after socket shutdown")
        sender.close()
        if encoder is not None:
            encoder.close()

    elapsed_ns = time.perf_counter_ns() - started
    if sender_thread.is_alive():
        raise TimeoutError("local socket sender did not finish within 10 seconds")
    if sender_errors:
        raise RuntimeError("local socket sender failed") from sender_errors[0]
    if bytes(decoded) != expected:
        raise ValueError("transport round-trip did not reproduce the synthetic payload")
    if wire_bytes == 0 and expected:
        raise ValueError("transport produced an empty encoded stream")

    return {
        "case": case_name,
        "codec": profile.name,
        "level": profile.level if profile.name != "none" else None,
        "raw_bytes": len(expected),
        "chunks_sent": len(chunks),
        "wire_bytes": wire_bytes,
        "ratio": wire_bytes / len(expected) if expected else 1.0,
        "first_event_ns": first_event_ns,
        "round_trip_ns": elapsed_ns,
        "sender_thread_cpu_ns": sender_metrics.get("cpu_ns"),
        "receiver_thread_cpu_ns": receiver_cpu_ns,
    }


def _sse_chunks(size: int) -> list[bytes]:
    frames = _sse_payload(size).split(b"\n\n")
    if not frames or frames[-1] != b"":
        raise ValueError("synthetic SSE payload did not end on an event boundary")
    return [frame + b"\n\n" for frame in frames[:-1]]


def _cases() -> dict[str, list[bytes]]:
    frame = next(case.data for case in payload_cases() if case.name == "frame-64KiB")
    return {
        "ipc-frame-64KiB": [frame],
        "sse-stream-256KiB": _sse_chunks(256 * 1024),
    }


def run_benchmark(iterations: int, campaigns: int = 3) -> dict[str, Any]:
    if iterations < 5:
        raise ValueError("use at least five measured repetitions")
    if campaigns < 1:
        raise ValueError("use at least one independent campaign")

    profiles = [
        _Profile("none", 0),
        _Profile("brotli", 4),
        _Profile("zstd", 1),
    ]
    measurements: list[dict[str, Any]] = []

    for case_name, chunks in _cases().items():
        samples: dict[str, list[dict[str, Any]]] = {profile.name: [] for profile in profiles}
        for campaign in range(1, campaigns + 1):
            for repetition in range(2 + iterations):
                profile_offset = (repetition + campaign - 1) % len(profiles)
                ordered_profiles = profiles[profile_offset:] + profiles[:profile_offset]
                for profile in ordered_profiles:
                    result = _transport_once(case_name, chunks, profile)
                    if repetition >= 2:
                        result["campaign"] = campaign
                        result["repetition"] = repetition - 1
                        samples[profile.name].append(result)

        for profile in profiles:
            runs = samples[profile.name]
            first_event_samples = [
                row["first_event_ns"]
                for row in runs
                if row["first_event_ns"] is not None
            ]
            measurements.append(
                {
                    "case": case_name,
                    "codec": profile.name,
                    "level": profile.level if profile.name != "none" else None,
                    "raw_bytes": runs[0]["raw_bytes"],
                    "chunks_sent": runs[0]["chunks_sent"],
                    "wire_bytes_median": int(statistics.median(row["wire_bytes"] for row in runs)),
                    "ratio_median": statistics.median(row["ratio"] for row in runs),
                    "first_event_p50_ns": _percentile(
                        first_event_samples,
                        0.50,
                    )
                    if case_name.startswith("sse-")
                    else None,
                    "first_event_p95_ns": _percentile(
                        first_event_samples,
                        0.95,
                    )
                    if case_name.startswith("sse-")
                    else None,
                    "round_trip_p50_ns": _percentile(
                        [row["round_trip_ns"] for row in runs], 0.50
                    ),
                    "round_trip_p95_ns": _percentile(
                        [row["round_trip_ns"] for row in runs], 0.95
                    ),
                    "sender_cpu_p50_ns": _percentile(
                        [row["sender_thread_cpu_ns"] for row in runs], 0.50
                    ),
                    "receiver_cpu_p50_ns": _percentile(
                        [row["receiver_thread_cpu_ns"] for row in runs], 0.50
                    ),
                    "runs": runs,
                }
            )

    return {
        "schema_version": 1,
        "generated_at_utc": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "environment": {
            "system": platform.system(),
            "release": platform.release(),
            "machine": platform.machine(),
            "processor": platform.processor() or None,
            "cpu_model": _cpu_model(),
            "python": platform.python_version(),
            "native_codecs": _native_versions(),
            "transport": "AF_UNIX SOCK_STREAM socketpair",
            "socket_chunk_bytes": _OUTPUT_BUFFER_BYTES,
            "flush_policy": "flush after each input chunk; each SSE event is one input chunk",
            "warmup_repetitions": 2,
            "measured_repetitions_per_campaign": iterations,
            "campaigns": campaigns,
            "execution_order": "serial; profile order rotates between repetitions",
            "measurement_note": (
                "includes Python ctypes, per-event compressor flushes, socketpair, "
                "and streaming decoder; "
                "excludes protocol framing, HTTP/TLS, and remote network"
            ),
        },
        "measurements": measurements,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--iterations", type=int, default=10)
    parser.add_argument("--campaigns", type=int, default=3)
    arguments = parser.parse_args()
    report = run_benchmark(arguments.iterations, arguments.campaigns)
    sys.stdout.write(json.dumps(report, indent=2, sort_keys=True) + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
