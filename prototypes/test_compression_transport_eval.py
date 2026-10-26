import unittest

from prototypes.compression_transport_eval import (
    _BrotliDecoder,
    _BrotliEncoder,
    _Profile,
    _ZstdDecoder,
    _ZstdEncoder,
    _transport_once,
)


class CompressionTransportEvaluationTests(unittest.TestCase):
    def _encode(self, codec: str, payload: bytes) -> bytes:
        encoder = _BrotliEncoder(4) if codec == "brotli" else _ZstdEncoder(1)
        try:
            return encoder.write(payload) + encoder.finish()
        finally:
            encoder.close()

    def _decode(self, codec: str, maximum_output_bytes: int):
        return (
            _BrotliDecoder(maximum_output_bytes)
            if codec == "brotli"
            else _ZstdDecoder(maximum_output_bytes)
        )

    def test_incremental_streams_round_trip_in_small_transport_chunks(self) -> None:
        payload = (b"synthetic event: response.output_text.delta\n\n" * 512) + b"done\n\n"

        for codec in ["brotli", "zstd"]:
            with self.subTest(codec=codec):
                encoded = self._encode(codec, payload)
                decoder = self._decode(codec, len(payload))
                decoded = bytearray()
                try:
                    for offset in range(0, len(encoded), 7):
                        decoded.extend(decoder.feed(encoded[offset : offset + 7]))
                    decoder.finish()
                finally:
                    decoder.close()

                self.assertEqual(bytes(decoded), payload)

    def test_streaming_socket_measurement_preserves_incremental_sse_event(self) -> None:
        chunks = [b"event: response.output_text.delta\ndata: {\"n\":1}\n\n"]
        chunks.extend(
            f'event: response.output_text.delta\ndata: {{"n":{index}}}\n\n'.encode()
            for index in range(2, 32)
        )

        for profile in [_Profile("none", 0), _Profile("brotli", 4), _Profile("zstd", 1)]:
            with self.subTest(codec=profile.name):
                if profile.name != "none":
                    encoder = (
                        _BrotliEncoder(profile.level)
                        if profile.name == "brotli"
                        else _ZstdEncoder(profile.level)
                    )
                    try:
                        self.assertTrue(encoder.write(chunks[0]))
                    finally:
                        encoder.close()

                result = _transport_once("sse-test", chunks, profile)
                self.assertEqual(result["raw_bytes"], sum(len(chunk) for chunk in chunks))
                self.assertGreater(result["wire_bytes"], 0)
                self.assertGreater(result["round_trip_ns"], 0)
                self.assertGreater(result["first_event_ns"], 0)

    def test_truncated_streams_are_rejected_at_end_of_input(self) -> None:
        payload = b"bounded synthetic stream" * 256

        for codec in ["brotli", "zstd"]:
            with self.subTest(codec=codec):
                encoded = self._encode(codec, payload)
                decoder = self._decode(codec, len(payload))
                try:
                    for offset in range(0, len(encoded) - 1, 11):
                        decoder.feed(encoded[offset : min(offset + 11, len(encoded) - 1)])
                    with self.assertRaises(ValueError):
                        decoder.finish()
                finally:
                    decoder.close()

    def test_invalid_stream_bytes_are_rejected(self) -> None:
        for codec in ["brotli", "zstd"]:
            with self.subTest(codec=codec):
                decoder = self._decode(codec, 4096)
                try:
                    with self.assertRaises(ValueError):
                        decoder.feed(b"\xff" * 64)
                finally:
                    decoder.close()

    def test_decompression_output_is_bounded_for_highly_compressible_input(self) -> None:
        payload = b"\x00" * (256 * 1024)

        for codec in ["brotli", "zstd"]:
            with self.subTest(codec=codec):
                encoded = self._encode(codec, payload)
                decoder = self._decode(codec, 32 * 1024)
                try:
                    with self.assertRaisesRegex(ValueError, "byte limit"):
                        for offset in range(0, len(encoded), 5):
                            decoder.feed(encoded[offset : offset + 5])
                finally:
                    decoder.close()


if __name__ == "__main__":
    unittest.main()
