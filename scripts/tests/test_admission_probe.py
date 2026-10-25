import importlib.util
import json
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location("admission_probe", Path(__file__).parents[1] / "measure-admission-probe.py")
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


class AdmissionProbeTests(unittest.TestCase):
    def executable(self, root, body):
        executable = Path(root) / "synthetic-probe"
        executable.write_text("#!/bin/sh\n" + body + "\n")
        executable.chmod(0o700)
        return executable

    def sample(self):
        return {
            "schema_version": 1, "scope": "sqlite-admission-control-plane-only",
            "iterations": 16, "nominal_bytes_per_intent": 1024 * 1024,
            "open_us": 1, "immediate_total_us": 2, "enqueue_total_us": 3, "claim_total_us": 4,
            "remaining_reserved_bytes": 0, "remaining_waiting": 0, "production_defaults_calibrated": False,
        }

    def test_accepts_only_the_exact_bounded_workload(self):
        sample = self.sample()
        self.assertEqual(probe.validate_probe(sample), sample)

    def test_rejects_unknown_fields_and_unbounded_or_wrong_typed_metrics(self):
        for key, value in [("unexpected", "private"), ("iterations", 17), ("open_us", True),
                           ("open_us", -1), ("open_us", 2**64), ("claim_total_us", "4"),
                           ("production_defaults_calibrated", True), ("remaining_waiting", 1)]:
            sample = self.sample()
            sample[key] = value
            with self.assertRaises(ValueError):
                probe.validate_probe(sample)
        with self.assertRaises(ValueError):
            probe.validate_probe([])

    def test_measures_an_actual_child_with_allowlisted_json_and_no_ambient_environment(self):
        with tempfile.TemporaryDirectory() as root:
            executable = self.executable(root, "test -z \"$CARAPANA_SYNTHETIC_AMBIENT\" || exit 1\nprintf '%s\\n' " + shlex.quote(json.dumps(self.sample())))
            with patch.dict("os.environ", {"CARAPANA_SYNTHETIC_AMBIENT": "must-not-forward"}):
                result = probe.measure(executable)
            self.assertEqual(result["probe"], self.sample())
            self.assertEqual(len(result["binary_sha256"]), 64)
            self.assertGreaterEqual(result["child_max_rss_bytes"], 0)
            self.assertTrue(result["rusage_includes_preexec"])

    def test_refuses_output_over_the_budget(self):
        with tempfile.TemporaryDirectory() as root:
            executable = self.executable(root, "printf '%09000d' 0")
            with self.assertRaises(ValueError):
                probe.measure(executable)

    def test_kills_and_reaps_the_owned_child_on_timeout(self):
        with tempfile.TemporaryDirectory() as root:
            executable = self.executable(root, "exec /bin/sleep 5")
            with patch.object(probe, "TIMEOUT", 0.05):
                with self.assertRaises(TimeoutError):
                    probe.measure(executable)

    def test_repository_rust_pin_wins_over_an_unavailable_global_override(self):
        root = Path(__file__).parents[2]
        pin_line = next(line for line in (root / "rust-toolchain.toml").read_text().splitlines() if line.startswith('channel = "'))
        pin = pin_line.split('"')[1]
        environment = dict(os.environ)
        environment["RUSTUP_TOOLCHAIN"] = "carapana-unavailable-synthetic"
        result = subprocess.run(["bash", str(root / "scripts/rust-toolchain-command.sh"), "rustc", "--version"],
                                env=environment, check=True, text=True, capture_output=True)
        self.assertTrue(result.stdout.startswith("rustc " + pin + " "))


if __name__ == "__main__":
    unittest.main()
