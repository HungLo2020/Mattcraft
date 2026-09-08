import json
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "Common"))
import canonical_recovery as recovery


class CanonicalRecoveryTests(unittest.TestCase):
    def trees(self, root):
        base, source = root / "base", root / "source"
        base.mkdir(); source.mkdir()
        for folder in (base, source):
            (folder / "empty").mkdir()
            (folder / "save").write_bytes(b"same save")
            for i in range(6):
                (folder / str(i)).write_bytes(b"baseline" if folder == base else bytes([i]))
        return base, source

    def test_all_six_differences_are_archived_and_source_is_never_deleted(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp); base, source = self.trees(root)
            result = recovery.prepare_recovery(base, source, root / "output")
            self.assertTrue(result["verified"])
            self.assertEqual([str(i) for i in range(6)], result["changed_files"])
            self.assertTrue((source / "save").is_file())
            with tarfile.open(result["archive"]) as archive:
                self.assertEqual(result["changed_files"], archive.getnames())
                for i in range(6):
                    self.assertEqual(bytes([i]), archive.extractfile(str(i)).read())
            self.assertEqual(result, json.loads((root / "output/canonical-recovery.json").read_text()))
            with self.assertRaises(ValueError):
                recovery.prepare_recovery(base, source, root / "output")

    def test_missing_file_symlink_or_live_change_cannot_publish_verified_receipt(self):
        for fault in ("missing", "symlink", "type", "directory", "changed"):
            with self.subTest(fault=fault), tempfile.TemporaryDirectory() as temp:
                root = Path(temp); base, source = self.trees(root)
                if fault == "missing":
                    (source / "save").unlink()
                elif fault == "symlink":
                    (source / "link").symlink_to(source / "save")
                elif fault == "type":
                    (source / "save").unlink()
                    (source / "save").mkdir()
                elif fault == "directory":
                    (source / "new-empty-directory").mkdir()
                original = recovery.inventory
                calls = 0
                def inventory(path):
                    nonlocal calls
                    calls += 1
                    if fault == "changed" and calls == 3:
                        (source / "save").write_bytes(b"changed during archive")
                    return original(path)
                with mock.patch.object(recovery, "inventory", side_effect=inventory), self.assertRaises(ValueError):
                    recovery.prepare_recovery(base, source, root / "output")
                self.assertFalse((root / "output/canonical-recovery.json").exists())
                self.assertTrue(source.is_dir())

    def test_roots_cannot_overlap_or_use_symlinks(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp); base, source = self.trees(root)
            alias = root / "alias"
            alias.symlink_to(source, target_is_directory=True)
            for other, output in ((alias, root / "output"), (base, root / "output"),
                                  (source, source / "output"), (source, base / "output")):
                with self.assertRaises(ValueError):
                    recovery.prepare_recovery(base, other, output)
