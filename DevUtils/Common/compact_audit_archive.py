"""Explicit, reviewable compaction of completed local graphics-audit archives.

Plan first; apply only the saved exact targets. No general repository cleanup.
Historical JSON is gzip-compressed with SHA256 verification, never discarded.
Recent rendering evidence and the canonical recovery base are kept untouched.
"""
import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
from concurrent.futures import ThreadPoolExecutor, as_completed

REPO = Path(__file__).resolve().parents[2]
ROOTS = (REPO / "logs/graphics-audit", REPO / "artifacts/graphics-captures")
BASE_RUN = ROOTS[0] / "dh-isolated-moon-r100"


def checked(path):
    p = Path(path)
    if p.is_symlink() or p.resolve() != p or not any(r in p.parents for r in ROOTS):
        raise ValueError("not an exact generated artifact child: " + str(p))
    if p == BASE_RUN or BASE_RUN in p.parents or p in BASE_RUN.parents:
        raise ValueError("canonical recovery base is protected")
    return p


def idle():
    for proc in Path('/proc').iterdir():
        if not proc.name.isdigit() or int(proc.name)==os.getpid():
            continue
        try:
            args=(proc/'cmdline').read_bytes().split(b'\0')
        except (OSError,PermissionError):
            continue
        executable=Path(args[0].decode(errors='replace')).name if args else ''
        launcher=executable.startswith('python') or executable in ('capture_runner.py','graphics_harness.py')
        if launcher and any(Path(a.decode(errors='replace')).name in ('capture_runner.py','graphics_harness.py') for a in args):
            raise RuntimeError('capture/harness process still alive: '+proc.name)


def signature(p):
    s=p.stat()
    return {'size':s.st_size,'mtime_ns':s.st_mtime_ns,'inode':s.st_ino}


def recent(p,root,keep_full_from_run=433):
    top=p.relative_to(root).parts[0]
    match=re.search(r'(?:^|-)r(\d+)(?:-|$)',top)
    return bool(match and int(match[1])>=keep_full_from_run)


def plan(keep_full_from_run=433):
    if keep_full_from_run < 1: raise ValueError('keep-full cutoff must be positive')
    idle()
    deletions=[]; compress=[]
    for root in ROOTS:
        for parent,dirs,files in os.walk(root,followlinks=False):
            parent=Path(parent)
            dirs[:]=[d for d in dirs if not (parent/d).is_symlink()]
            if parent==BASE_RUN or BASE_RUN in parent.parents:
                dirs[:]=[]; continue
            for d in list(dirs):
                p=parent/d
                copied=(d in ('.canonical-fixtures','.tmp') or d.startswith(('game_dir_','region_validation_game_'))
                    or p==ROOTS[0]/'regular-fixture-control-r111/run')
                if copied:
                    checked(p)
                    deletions.append({'path':str(p),'signature':signature(p),'reason':'completed generated game/world copy'})
                    dirs.remove(d)
            for name in files:
                p=parent/name
                if p.is_symlink() or recent(p,root,keep_full_from_run): continue
                if p.suffix in ('.json','.jsonl') and p.stat().st_size>=1024**2:
                    checked(p)
                    compress.append({'path':str(p),'signature':signature(p),'reason':'lossless historical diagnostic archive'})
    return {'schema':'explicit-audit-compaction-v1','protected_base':str(BASE_RUN),
        'recent_full_evidence':f'run IDs >={keep_full_from_run}','delete':deletions,'compress':compress}


def digest(stream):
    return hashlib.file_digest(stream,'sha256').hexdigest()


def compress_one(item):
    p=checked(item['path']); target=p.with_name(p.name+'.gz')
    with p.open('rb') as source: before=digest(source)
    if not target.exists():
        with p.open('rb') as source, target.open('xb') as raw:
            with gzip.GzipFile(filename=p.name,mode='wb',fileobj=raw,compresslevel=1,mtime=0) as out:
                shutil.copyfileobj(source,out,1024**2)
    with gzip.open(target,'rb') as source: after=digest(source)
    if before!=after or signature(p)!=item['signature']:
        raise ValueError('compression verification failed; original retained: '+str(p))
    p.unlink()
    return {'compressed':str(p),'archive':str(target),'sha256':before,
            'before_bytes':item['signature']['size'],'after_bytes':target.stat().st_size}


def apply(manifest,receipt):
    idle()
    if manifest.get('schema')!='explicit-audit-compaction-v1': raise ValueError('unknown plan')
    # Validate every target before touching any of them. Refuse changed plans.
    for item in manifest['delete']+manifest['compress']:
        p=checked(item['path'])
        if signature(p)!=item['signature']: raise ValueError('target changed: '+str(p))
    with receipt.open('x',encoding='utf-8') as log:
        def record(value):
            log.write(json.dumps(value,sort_keys=True)+'\n'); log.flush()
        for item in manifest['delete']:
            idle(); p=checked(item['path'])
            shutil.rmtree(p)
            record({'deleted':str(p),'reason':item['reason'],
                    'recovery':'canonical base retained; obsolete run-specific copies not guaranteed recoverable'})
        with ThreadPoolExecutor(max_workers=4) as pool:
            futures=[pool.submit(compress_one,item) for item in manifest['compress']]
            for index,future in enumerate(as_completed(futures)):
                record(future.result())
                if index%100==0: print(f'Verified {index+1}/{len(manifest["compress"])} diagnostic archives',flush=True)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--plan',type=Path,required=True)
    parser.add_argument('--apply',action='store_true')
    parser.add_argument('--receipt',type=Path)
    parser.add_argument('--keep-full-from-run',type=int,default=433,
                        help='Retain uncompressed evidence from this active run ID onward; older JSON is archived, not lost.')
    args=parser.parse_args()
    if args.apply:
        if args.receipt is None: parser.error('--apply requires --receipt')
        apply(json.loads(args.plan.read_text()),args.receipt)
    else:
        result=plan(args.keep_full_from_run)
        with args.plan.open('x',encoding='utf-8') as out: json.dump(result,out,indent=2)
        print(json.dumps({'delete_directories':len(result['delete']),'compress_files':len(result['compress']),
            'compress_input_gib':sum(i['signature']['size'] for i in result['compress'])/1024**3}))
