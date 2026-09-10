"""Frozen OpenGL r588: transparent opaque-material plate occludes green handle.

Fixed camera/scale3, shield-alpha-occlusion resource pack, unenchanted shield.
Reuse the existing shield tolerance and regions without fitting the candidate.
"""
from PIL import Image
from shield_item_reference import _compare_images

PROBES = [
    ['gui_hidden_handle_top',397,683,[(45,45,11),(53,55,13),(52,54,13),
        (37,38,9),(43,43,10),(40,39,10),(45,49,12),(40,43,11),(35,37,10)]],
    ['gui_hidden_handle_bottom',398,687,[(40,42,11),(51,56,14),(52,53,13),
        (46,50,12),(42,43,10),(41,41,10),(40,40,9),(42,43,10),(43,44,11)]],
    ['gui_hidden_handle_left',394,688,[(51,52,12),(38,39,9),(55,58,13),
        (38,38,9),(37,38,9),(45,47,11),(39,41,9),(44,46,11),(39,40,9)]],
    ['held_top',800,475,[(39,236,79)]*9],
    ['held_left',740,520,[(22,134,45)]*9],
    ['held_plate',800,600,[(22,133,45)]*9],
    ['held_right',1150,610,[(23,133,45)]*9],
]


def compare_images(baseline, current):
    return _compare_images(baseline, current, PROBES, "shield-alpha-occlusion-v1")


def compare_paths(baseline, current):
    with Image.open(baseline) as a, Image.open(current) as b:
        return compare_images(a, b)
