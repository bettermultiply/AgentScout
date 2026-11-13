"""Regression for recovery initialization changing an existing seeded world.

Run with the configured NLE Python; no model or tool-result substitution.
"""
import importlib.util
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

import gymnasium as gym
import nle  # noqa: F401

spec = importlib.util.spec_from_file_location(
    "nle_bridge", Path(__file__).resolve().parents[1] / "examples/nle-runtime-bridge.py"
)
bridge = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = bridge
spec.loader.exec_module(bridge)


class Initialization(unittest.TestCase):
    def test_seed_does_not_enable_independent_level_generator(self):
        with patch.object(bridge.secrets, "randbits", return_value=11):
            world = bridge.new_world(gym, "NetHackScore-v0", seed=7)
        try:
            self.assertIsNone(world.initial["seeds"][3])
            self.assertEqual(world.observation["blstats"][:2], [26, 5])
            for action in [1, 2, 3, 4]:
                bridge.step(world, action)
            restored = bridge.restore_world(gym, {
                "initial": world.initial, "actions": world.actions,
                "checks": world.checks, "state": world.state,
            })
            try:
                self.assertEqual(restored.checks, world.checks)
                self.assertEqual(restored.observation, world.observation)
            finally:
                restored.environment.close()
        finally:
            world.environment.close()


if __name__ == "__main__":
    unittest.main()
