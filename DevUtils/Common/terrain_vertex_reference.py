"""Strict comparison of bounded GPU vertex observations; never a visual parity gate."""
import math
import struct


def floating(word):
    return struct.unpack('<f', struct.pack('<I', word))[0]


def decode(count, truncated, records):
    if type(count) is not int or not 0 < count <= 128 or truncated is not False:
        raise ValueError('missing or truncated GPU vertex observation')
    if len(records) != count:
        raise ValueError('GPU vertex record count disagrees with payload')
    corners = {}
    for record in records:
        if len(record) != 12 or any(type(w) is not int or not 0 <= w <= 0xffffffff for w in record):
            raise ValueError('invalid GPU vertex record')
        world = tuple(floating(w) for w in record[2:5])
        if not all(math.isfinite(x) for x in world):
            raise ValueError('nonfinite GPU position')
        # Group matching fixture corners without treating rounded values as
        # equality evidence: compare the original raw bits below.
        corner = tuple(round(x, 2) for x in world)
        entry = corners.setdefault(corner, {'world': set(), 'clip': set(), 'uv': set()})
        entry['world'].add(tuple(record[2:6]))
        entry['clip'].add(tuple(record[6:10]))
        entry['uv'].add(tuple(record[10:12]))
    expected = {(x, y, z) for x in (-3.5, -2.5) for y in (-0.62, 0.38) for z in (-1.5, -0.5)}
    if set(corners) != expected:
        raise ValueError('GPU observation does not cover exactly all eight fixture corners')
    return corners


def compare(left, right):
    return {
        'corner_count': len(left),
        'world_bits_equal': all(left[k]['world'] == right[k]['world'] for k in left),
        'clip_bits_equal': all(left[k]['clip'] == right[k]['clip'] for k in left),
        'uv_bits_equal': all(left[k]['uv'] == right[k]['uv'] for k in left),
        'different_clip_corners': [k for k in left if left[k]['clip'] != right[k]['clip']],
        'visual_parity_accepted': False,
    }


def inspect_run(root, baseline):
    import ast
    import json
    import re
    from PIL import Image, ImageChops
    modes = ('current-rust-vulkan-shaders-off', 'frozen-opengl-shaders-off')
    dirs = [root / mode / 'capture/run-01/capture' for mode in modes]
    logs = list(dirs[0].glob('runClient*.log'))
    if len(logs) != 1: raise ValueError('expected one Rust runtime log')
    rust = []
    for line in logs[0].read_text().splitlines():
        match = re.search(r'terrain-vertex-observation submission=(\d+) count=(\d+) truncated=(true|false) records=(\[.*\])', line)
        if match and int(match[2]):
            rows = ast.literal_eval(match[4])
            rust.append({'submission': int(match[1]), 'count': int(match[2]),
                         'truncated': match[3] == 'true', 'records': rows})
    frozen = []
    for path in dirs[1].glob('static_terrain_parity_diagnostics*.jsonl'):
        for line in path.open():
            receipt = (json.loads(line).get('gpuTerrainUv') or {}).get('vertexObservation')
            if receipt: frozen.append(receipt)
    if len(rust) != 5 or len(frozen) != 5:
        raise ValueError(f'expected five complete observation pairs, got {len(rust)}/{len(frozen)}')
    comparisons = [compare(decode(a['count'], a['truncated'], a['records']),
                           decode(b['count'], b['truncated'], b['records'])) for a, b in zip(rust, frozen)]
    pixels = []
    for name in ('01_initial', '02_right', '03_left', '04_return', '05_initial'):
        crops = []
        unchanged = []
        for mode, directory in zip(modes, dirs):
            current = list(directory.glob('deterministic_camera_capture_*/' + name + '.png'))
            old = list((baseline / mode / 'capture/run-01/capture').glob('deterministic_camera_capture_*/' + name + '.png'))
            if len(current) != 1 or len(old) != 1: raise ValueError('missing or ambiguous screenshot')
            a, b = [Image.open(p).convert('RGB').crop((700,300,765,370)) for p in (current[0],old[0])]
            crops.append(a)
            unchanged.append(ImageChops.difference(a,b).getbbox() is None)
        difference = list(ImageChops.difference(*crops).get_flattened_data())
        pixels.append({'capture': name, 'rust_roi_unchanged': unchanged[0], 'frozen_roi_unchanged': unchanged[1],
                       'changed_pixels': sum(any(p) for p in difference),
                       'max_channel_error': max(max(p) for p in difference),
                       'pixels_over_existing_limit_6': sum(max(p) > 6 for p in difference)})
    return {'schema': 'terrain-gpu-all-vertex-observation-v1', 'run': str(root), 'baseline': str(baseline),
            'region': [700,300,765,370], 'rust_observations': rust, 'frozen_observations': frozen,
            'vertex_comparisons': comparisons, 'pixel_comparisons': pixels, 'visual_parity_accepted': False}


if __name__ == '__main__':
    import argparse
    import json
    from pathlib import Path
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run', type=Path, required=True)
    parser.add_argument('--baseline', type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(inspect_run(args.run.resolve(), args.baseline.resolve()), indent=2))
