import copy
from pathlib import Path
import sys
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "Common"))
from graphics_harness import cow_outline_fixture_equivalence


class CowOutlineReferenceTest(unittest.TestCase):
    def fixture(self):
        return {"cowOutlineFixture": dict(requested=True, ready=True, entityPresent=True,
            glowing=True, invisible=False, invisibleRequested=False, x=1.0, y=2.0, z=3.0,
            modelObservation=dict(bodyRot=0.0, yRot=0.0, xRot=0.0, walkAnimationSpeed=0.0,
                pose=[1.0] * 16, parts={"root": [0.0] * 9}))}

    def test_random_body_rotation_or_missing_observation_is_not_visual_parity(self):
        baseline = self.fixture()
        self.assertTrue(cow_outline_fixture_equivalence(baseline, copy.deepcopy(baseline))["passed"])
        for key, value in (("bodyRot", 2.8125), ("pose", []), ("parts", {})):
            changed = copy.deepcopy(baseline)
            changed["cowOutlineFixture"]["modelObservation"][key] = value
            self.assertFalse(cow_outline_fixture_equivalence(baseline, changed)["passed"])
        del baseline["cowOutlineFixture"]["modelObservation"]
        self.assertFalse(cow_outline_fixture_equivalence(baseline, baseline)["passed"])

    def test_invisible_control_and_missing_counterpart_cannot_be_accepted(self):
        baseline = self.fixture()
        changed = copy.deepcopy(baseline)
        changed["cowOutlineFixture"].update(invisible=True, invisibleRequested=True)
        self.assertFalse(cow_outline_fixture_equivalence(baseline, changed)["passed"])
        self.assertFalse(cow_outline_fixture_equivalence(baseline, {})["passed"])
        self.assertTrue(cow_outline_fixture_equivalence({}, {})["passed"])

if __name__ == "__main__":
    unittest.main()
