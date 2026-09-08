"""Prepare a verified recovery delta for a completed canonical capture fixture.

Never deletes the source. Callers must separately establish that its run is
terminal before reclaiming it. Both inventories must have identical paths;
added/removed files or symlinks fail closed rather than produce a partial delta.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import tarfile


def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def inventory(root: Path) -> dict[str, dict]:
    result = {}
    if root.is_symlink() or not root.is_dir():
        raise ValueError("inventory root must be a real directory")
    for path in sorted(root.rglob("*")):
        name = path.relative_to(root).as_posix()
        if path.is_symlink():
            raise ValueError("symlink in canonical fixture: " + name)
        if path.is_dir():
            result[name] = {"kind": "directory"}
        elif path.is_file():
            result[name] = {"kind": "file", "size": path.stat().st_size, "sha256": digest(path)}
        else:
            raise ValueError("unsupported canonical entry: " + name)
    return result


def prepare_recovery(base: Path, source: Path, output: Path) -> dict:
    if base.is_symlink() or source.is_symlink() or output.is_symlink():
        raise ValueError("recovery roots must not be symlinks")
    base, source, output = base.resolve(), source.resolve(), output.resolve()
    if base == source or base in source.parents or source in base.parents:
        raise ValueError("base and source must be separate directories")
    if output == base or output == source or base in output.parents or source in output.parents:
        raise ValueError("recovery output must be outside both inventories")
    base_entries, source_entries = inventory(base), inventory(source)
    if base_entries.keys() != source_entries.keys() or any(
            base_entries[name]["kind"] != entry["kind"] for name, entry in source_entries.items()):
        raise ValueError("canonical paths differ; added/removed/type-changed entries require explicit recovery")
    changed = sorted(name for name in source_entries if source_entries[name] != base_entries[name])
    output.mkdir(parents=True, exist_ok=True)
    archive = output / "canonical-recovery-overlay.tar.gz"
    receipt = output / "canonical-recovery.json"
    if archive.exists() or receipt.exists():
        raise ValueError("refusing to overwrite existing recovery evidence")
    with tarfile.open(archive, "x:gz") as bundle:
        for name in changed:
            bundle.add(source / name, arcname=name, recursive=False)
    with tarfile.open(archive, "r:gz") as bundle:
        if bundle.getnames() != changed:
            raise ValueError("archive inventory differs from complete delta")
        for member in bundle.getmembers():
            if not member.isfile():
                raise ValueError("delta contains a non-file entry")
            stream = bundle.extractfile(member)
            if stream is None:
                raise ValueError("missing archive payload")
            with stream:
                value = hashlib.file_digest(stream, "sha256").hexdigest()
            if value != source_entries[member.name]["sha256"] or member.size != source_entries[member.name]["size"]:
                raise ValueError("archive payload differs from source")
    # A successful archive read alone is not proof if either tree changed
    # during preparation. No verified receipt is published in that case.
    if inventory(source) != source_entries or inventory(base) != base_entries:
        raise ValueError("canonical inventory changed while archiving")
    result = {"schema": "verified-canonical-recovery-v1", "verified": True,
              "base": str(base), "source": str(source), "archive": str(archive),
              "archive_sha256": digest(archive), "changed_files": changed,
              "base_inventory": base_entries, "source_inventory": source_entries,
              "source_deleted": False}
    with receipt.open("x", encoding="utf-8") as stream:
        json.dump(result, stream, sort_keys=True, indent=2)
        stream.write("\n")
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", type=Path, required=True)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = prepare_recovery(args.base, args.source, args.output)
    print(json.dumps({key: result[key] for key in ("verified", "changed_files", "archive", "source_deleted")}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
