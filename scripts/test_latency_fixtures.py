"""Offline synthetic fixture tests; no tokenizer install, key, or network needed."""
import json
from pathlib import Path
import unittest

from generate_latency_fixtures import TARGETS, encoded_history, synthetic_history


class SyntheticFixturesTest(unittest.TestCase):
    def test_fixed_generator_is_synthetic_and_deterministic(self):
        history = synthetic_history(2, 3)
        self.assertEqual(history, [
            "git log --oneline --max-count=8 -- synthetic/fixture_0000.txt",
            "git log --oneline --max-count=8 -- synthetic/fixture_0001.txt",
            "echo synthetic synthetic synthetic",
        ])
        self.assertEqual(json.loads(encoded_history(history)), history)
        self.assertEqual(synthetic_history(0, 0), [])

    def test_bundled_manifest_has_bounded_token_targets_and_provenance(self):
        manifest_path = Path(__file__).resolve().parents[1] / "examples" / "latency_fixture_manifest.json"
        manifest = json.loads(manifest_path.read_text())
        self.assertEqual(manifest["schema"], 1)
        self.assertEqual(len(manifest["tokenizer_sha256"]), 64)
        self.assertEqual(tuple(t["nominal_history_tokens"] for t in manifest["targets"]), TARGETS)
        for target in manifest["targets"]:
            self.assertEqual(target["nominal_history_tokens"], target["local_history_json_tokens"])
            self.assertLessEqual(target["full_lines"], 1000)
            self.assertLessEqual(target["tail_terms"], 128)
            text = encoded_history(synthetic_history(target["full_lines"], target["tail_terms"]))
            self.assertLess(len(text.encode()), 64 * 1024)


if __name__ == "__main__":
    unittest.main()
