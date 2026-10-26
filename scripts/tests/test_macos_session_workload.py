import importlib.util
from pathlib import Path
import sys
import unittest


MODULE_PATH = Path(__file__).parents[1] / "measure-macos-session-workload.py"
SPEC = importlib.util.spec_from_file_location("macos_session_workload", MODULE_PATH)
benchmark = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = benchmark
SPEC.loader.exec_module(benchmark)


class MacosSessionWorkloadTests(unittest.TestCase):
    def test_workload_counts_bound_the_synthetic_queue_volume(self):
        typical = benchmark.Workload("typical", 4, 8, 1024, 1)
        burst = benchmark.Workload("burst", 8, 4, 8192, 4)

        self.assertEqual(typical.request_count, 32)
        self.assertEqual(typical.payload_bytes, 32768)
        self.assertEqual(burst.request_count, 32)
        self.assertEqual(burst.payload_bytes, 262144)

    def test_percentile_uses_ordered_samples(self):
        self.assertEqual(benchmark.percentile([5, 1, 4, 2, 3], 0.50), 3)
        self.assertEqual(benchmark.percentile([5, 1, 4, 2, 3], 0.95), 5)

    def test_process_registry_tracks_only_active_cli_children(self):
        registry = benchmark.ProcessRegistry(10)
        registry.add_client(11)
        registry.add_client(12)
        self.assertEqual(set(registry.process_ids()), {10, 11, 12})

        registry.remove_client(11)

        self.assertEqual(registry.process_ids(), [10, 12])


if __name__ == "__main__":
    unittest.main()
