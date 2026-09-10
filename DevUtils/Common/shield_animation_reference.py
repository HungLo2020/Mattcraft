"""Shield animation pixels independently captured from Frozen Java OpenGL.

r600: discrete frame1/sheet0/subframe1; r601: interpolated at the same phase.
r603: interpolated frame2/sheet2/subframe1 (phase nine, repeated source frame).
r604: interpolated cycle boundary, frame0/sheet2/subframe0 after eight cycles.
These are local pixel diagnostics, NOT complete animation admission. Source
upload/presentation correlation and additional phases are still required.
Coordinates, 3x3 samples, regions, and tolerance are the existing shield oracle's.
"""
from PIL import Image
from shield_item_reference import BASE_PROBES, _compare_images

FROZEN_PHASE_FOUR = {
    False: [(207, 59, 30)] * 3 + [(220, 63, 32), (125, 36, 18),
                                  (125, 36, 18), (124, 36, 18)],
    True: [(175, 62, 66)] * 3 + [(186, 66, 70), (106, 38, 40),
                                 (106, 38, 40), (105, 38, 40)],
}
FROZEN_PHASE_NINE_INTERPOLATED = [(37, 133, 133)] * 3 + [
    (39, 142, 142), (22, 81, 81), (22, 81, 81), (23, 80, 80)]
FROZEN_PHASE_ZERO_INTERPOLATED = [(44, 74, 207)] * 3 + [
    (47, 79, 221), (27, 45, 125), (27, 45, 125), (27, 45, 124)]


def compare_phase_four_images(baseline, current, *, interpolated):
    return compare_phase_images(baseline, current, interpolated=interpolated, phase=4)


def compare_phase_images(baseline, current, *, interpolated, phase):
    if type(interpolated) is not bool:
        raise ValueError("an explicit interpolation policy is required")
    if type(phase) is not int or (phase != 4 and not (phase in (0, 9) and interpolated)):
        raise ValueError("no independent Frozen pixel oracle for this phase")
    colors = (FROZEN_PHASE_FOUR[interpolated] if phase == 4 else
              FROZEN_PHASE_NINE_INTERPOLATED if phase == 9 else FROZEN_PHASE_ZERO_INTERPOLATED)
    probes = [(name, x, y, [color] * 9)
              for (name, x, y, _), color in zip(BASE_PROBES, colors)]
    result = _compare_images(baseline, current, probes, "shield-animation-phase-pixels-v1")
    result.update(phase=phase, duration=17, interpolated=interpolated,
                  capability_admitted=False)
    return result


def compare_phase_four_paths(baseline, current, *, interpolated):
    return compare_phase_paths(baseline, current, interpolated=interpolated, phase=4)


def compare_phase_paths(baseline, current, *, interpolated, phase):
    with Image.open(baseline) as frozen, Image.open(current) as rust:
        return compare_phase_images(frozen, rust, interpolated=interpolated, phase=phase)
