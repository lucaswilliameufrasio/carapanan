import json
import unittest

from prototypes.compression_eval import (
    _brotli_codec,
    _sse_payload,
    _zstd_codec,
    payload_cases,
)


class CompressionEvaluationTests(unittest.TestCase):
    def test_payload_cases_are_bounded_and_valid_json_when_applicable(self) -> None:
        cases = {case.name: case for case in payload_cases()}

        self.assertEqual(len(cases["event-256B"].data), 256)
        self.assertEqual(len(cases["envelope-4KiB"].data), 4 * 1024)
        self.assertEqual(len(cases["snapshot-32KiB"].data), 32 * 1024)
        self.assertEqual(len(cases["frame-64KiB"].data), 64 * 1024)
        self.assertEqual(len(cases["opaque-binary-32B"].data), 32)
        self.assertGreaterEqual(len(cases["diff-32KiB"].data), 32 * 1024)
        self.assertGreaterEqual(len(cases["sse-stream-256KiB"].data), 256 * 1024)

        for case_name in ["event-256B", "envelope-4KiB", "snapshot-32KiB", "frame-64KiB"]:
            with self.subTest(case=case_name):
                json.loads(cases[case_name].data)

        json.loads(cases["entropy-256B"].data)
        json.loads(cases["entropy-64KiB"].data)

    def test_sse_payload_is_a_sequence_of_complete_synthetic_events(self) -> None:
        frames = _sse_payload(4096).split(b"\n\n")
        self.assertGreater(len(frames), 1)
        self.assertEqual(frames[-1], b"")

        self.assertEqual(frames[-2].splitlines()[0], b"event: response.completed")
        for index, frame in enumerate(frames[:-2]):
            lines = frame.splitlines()
            self.assertEqual(lines[0], b"event: response.output_text.delta")
            self.assertTrue(lines[1].startswith(b"data: "))
            event = json.loads(lines[1][len(b"data: ") :])
            self.assertEqual(event["sequence_number"], index)

    def test_codecs_round_trip_synthetic_bytes(self) -> None:
        payload = b'{"synthetic":true,"state":"paused"}' * 64
        small_incompressible = next(
            case.data for case in payload_cases() if case.name == "opaque-binary-32B"
        )

        for codec_name, codec in [("brotli", _brotli_codec), ("zstd", _zstd_codec)]:
            with self.subTest(codec=codec_name):
                _, _, operations = codec(payload, 4 if codec_name == "brotli" else 1)
                compress, prepare_decompress = operations
                compressed_size = compress()
                self.assertLess(compressed_size, len(payload))

                decompress, verify = prepare_decompress(compressed_size)
                self.assertEqual(decompress(), len(payload))
                self.assertTrue(verify())

                _, _, small_operations = codec(
                    small_incompressible,
                    4 if codec_name == "brotli" else 1,
                )
                self.assertGreater(small_operations[0](), len(small_incompressible))


if __name__ == "__main__":
    unittest.main()
