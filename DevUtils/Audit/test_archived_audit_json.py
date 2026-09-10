import gzip
import json
from pathlib import Path
import sys
import tempfile
import unittest
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'Common'))
from graphics_harness import read_json, deterministic_capture_document


class ArchivedAuditJsonTest(unittest.TestCase):
    def test_compressed_declared_capture_preserves_original_manifest_paths(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary); capture=root/'capture'; capture.mkdir()
            doc=capture/'deterministic_camera_capture_test.json'
            artifact=root/'graphics_audit_artifact.json'
            for path,value in ((doc,{'captures':[{'poseName':'initial'}]}),
                (artifact,{'capture':{'files':{'deterministic':str(doc)}}})):
                with gzip.open(str(path)+'.gz','wt') as out: json.dump(value,out)
            self.assertEqual({'captures':[{'poseName':'initial'}]},deterministic_capture_document(artifact))

    def test_corrupt_plain_evidence_cannot_fall_back_to_old_compressed_copy(self):
        with tempfile.TemporaryDirectory() as temporary:
            path=Path(temporary)/'evidence.json'
            with gzip.open(str(path)+'.gz','wt') as out: json.dump({'passed':True},out)
            path.write_text('corrupt current evidence')
            self.assertIsNone(read_json(path))

    def test_missing_corrupt_and_non_object_archives_fail_closed(self):
        with tempfile.TemporaryDirectory() as temporary:
            path=Path(temporary)/'evidence.json'
            self.assertIsNone(read_json(path))
            Path(str(path)+'.gz').write_bytes(b'not gzip')
            self.assertIsNone(read_json(path))
            with gzip.open(str(path)+'.gz','wt') as out: json.dump([],out)
            self.assertIsNone(read_json(path))


if __name__=='__main__': unittest.main()
