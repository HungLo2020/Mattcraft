import gzip
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'Common'))
import compact_audit_archive as archive


class AuditArchiveCompactionTest(unittest.TestCase):
    def test_protected_base_recent_evidence_and_lossless_old_metadata(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary); roots=(root/'audit',root/'captures')
            for r in roots: r.mkdir()
            base=roots[0]/'base'; base.mkdir()
            copied=roots[0]/'old-r20/.canonical-fixtures'; copied.mkdir(parents=True)
            (copied/'world').write_bytes(b'generated copy')
            protected=base/'.canonical-fixtures'; protected.mkdir()
            (protected/'world').write_bytes(b'authoritative base')
            old=roots[0]/'old-r20/evidence.json'; payload={'diagnostic':'x'*1024**2}
            old.write_text(json.dumps(payload))
            recent=roots[0]/'active-r464'; recent.mkdir()
            recent_file=recent/'evidence.json'; recent_file.write_text(json.dumps(payload))
            with patch.object(archive,'ROOTS',roots),patch.object(archive,'BASE_RUN',base),patch.object(archive,'idle'):
                plan=archive.plan()
                self.assertEqual([str(copied)],[x['path'] for x in plan['delete']])
                self.assertEqual([str(old)],[x['path'] for x in plan['compress']])
                archive.apply(plan,root/'receipt.jsonl')
                self.assertFalse(copied.exists())
                self.assertTrue((protected/'world').exists())
                self.assertTrue(recent_file.exists())
                with gzip.open(str(old)+'.gz','rt') as stream: self.assertEqual(payload,json.load(stream))
                self.assertFalse(old.exists())
                for invalid in (roots[0],base,protected,root/'outside'):
                    with self.assertRaises(ValueError): archive.checked(invalid)

    def test_changed_file_rejects_entire_plan_before_any_delete(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary); roots=(root/'audit',root/'captures')
            for r in roots: r.mkdir()
            copied=roots[0]/'old-r20/.tmp'; copied.mkdir(parents=True)
            old=roots[0]/'old-r20/evidence.json'; old.write_text('x'*1024**2)
            with patch.object(archive,'ROOTS',roots),patch.object(archive,'BASE_RUN',roots[0]/'base'),patch.object(archive,'idle'):
                plan=archive.plan(); old.write_text('changed')
                with self.assertRaises(ValueError): archive.apply(plan,root/'receipt.jsonl')
                self.assertTrue(copied.exists())


if __name__=='__main__': unittest.main()
