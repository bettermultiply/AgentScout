"""Reproduce full-enumeration identity changes without calling any model."""
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("decisions", Path(__file__).resolve().parents[1] / "assets/policy/pi-tool/decisions.py")
policy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(policy)


class ProposalOrder(unittest.TestCase):
    def test_selection_keeps_configured_order(self):
        for selected in [["blocked", "moved"], ["blocked"]]:
            with patch("secrets.SystemRandom") as rng:
                rng.return_value.sample.return_value = selected
                actual = policy.predict({}, policy_config={"enum_values": ["moved", "blocked"], "candidates": ["moved", "blocked"], "sample_count": len(selected)}, capabilities={})
            self.assertEqual([p["payload"] for p in actual["proposals"]], [v for v in ["moved", "blocked"] if v in selected])


if __name__ == "__main__":
    unittest.main()
