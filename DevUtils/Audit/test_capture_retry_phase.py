import inspect
import os
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'Common'))
import graphics_harness as harness


class CaptureRetryPhaseTest(unittest.TestCase):
    def test_previous_exit_cannot_start_new_run_finalization(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            previous = root / 'meta_previous.txt'
            previous.write_text('run_id=previous\nexit_code=0\n')
            prior = frozenset(root.glob('meta_*.txt'))
            self.assertEqual('artifact-finalization', harness.live_capture_phase(root, 'capture')[0])
            self.assertEqual('startup', harness.live_capture_phase(root, 'capture', prior_metadata=prior)[0])
            current = root / 'meta_current.txt'
            current.write_text('run_id=current\ngradle_pid=123\n')
            os.utime(previous, (1, 1))
            os.utime(current, (2, 2))
            self.assertEqual('process-launched', harness.live_capture_phase(root, 'capture', prior_metadata=prior)[0])
            current.write_text('run_id=current\nexit_code=0\n')
            self.assertEqual('artifact-finalization', harness.live_capture_phase(root, 'capture', prior_metadata=prior)[0])

    def test_parent_snapshots_prior_metadata_before_launch_and_resets_phase_timer(self):
        source = inspect.getsource(harness.run_mode)
        snapshot = source.index('prior_metadata = frozenset(')
        self.assertLess(snapshot, source.index('process = subprocess.Popen(', snapshot))
        self.assertIn('live_capture_phase(capture_dir, tool_kind, prior_metadata=prior_metadata)', source)
        self.assertIn('else:\n                    artifact_finalization_started = None', source)


if __name__ == '__main__':
    unittest.main()
