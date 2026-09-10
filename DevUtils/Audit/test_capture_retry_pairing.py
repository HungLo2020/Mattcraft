import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'Common'))
import graphics_harness as harness


class CaptureRetryPairingTest(unittest.TestCase):
    def test_image_and_semantics_follow_declared_run_not_sorted_retry(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            capture = root / 'capture'
            capture.mkdir()
            artifact = root / 'graphics_audit_artifact.json'
            manifests = []
            images = []
            for stamp in ('001', '002', '003'):
                image = capture / (stamp + '.png')
                image.touch()
                images.append(image)
                manifest = capture / ('deterministic_camera_capture_' + stamp + '.json')
                manifest.write_text(json.dumps({'run': stamp, 'captures': [{'screenshot': str(image)}]}))
                manifests.append(manifest)
            artifact.write_text(json.dumps({'capture': {'files': {'deterministic': str(manifests[1])}}}))
            self.assertEqual('002', harness.deterministic_capture_document(artifact)['run'])
            self.assertEqual(images[1], harness.deterministic_initial_frame_path(artifact))
            # Missing current evidence must not borrow any older/newer retry.
            images[1].unlink()
            self.assertIsNone(harness.deterministic_initial_frame_path(artifact))
            manifests[1].unlink()
            self.assertIsNone(harness.deterministic_capture_document(artifact))
            self.assertIsNone(harness.deterministic_initial_frame_path(artifact))
            artifact.write_text('{}')
            self.assertIsNone(harness.deterministic_capture_document(artifact))
            manifests[2].unlink()
            self.assertEqual('001', harness.deterministic_capture_document(artifact)['run'])


if __name__ == '__main__':
    unittest.main()
