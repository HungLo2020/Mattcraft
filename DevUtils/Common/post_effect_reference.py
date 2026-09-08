"""Independent CPU image model of Frozen Java OpenGL's bundled Spider graph.

This is test/diagnostic code, never a renderer fallback. The equations and
constants are from Frozen's spider.json, rotscale.vsh, spiderclip.fsh and
box_blur.fsh. Keep every intermediate RGBA8 conversion and unclamped mix
weight: the GLSL clamp(scaledCoord, ...) expression does not assign its result.
"""

import math
from PIL import Image, ImageFilter


def spider_reference_image(image, red_multiplier=1.0):
    if red_multiplier not in (1.0, 0.5):
        raise ValueError("Spider reference supports only bundled and dim-red fixtures")
    image = image.convert("RGB")
    width, height = image.size
    if min(width, height) < 1 or width * height > 4_194_304:
        raise ValueError("Spider reference requires a bounded nonempty image")

    def blur(radius):
        # The shader's half-texel linear samples reduce exactly to a clamped
        # uniform (2*radius+1)-tap filter. Each directional pass writes RGBA8.
        return image.filter(ImageFilter.BoxBlur((radius, 0))).filter(
            ImageFilter.BoxBlur((0, radius)))

    large = blur(15)
    small = blur(7)
    # (input, scale, offset, angle, scissor, vignette), in ordered graph order.
    passes = [
        (image, (1.25, 2.0), (-0.125, -0.1), 0.0,
         (0.0, 0.0, 1.0, 1.0), (0.1, 0.1, 0.9, 0.9)),
        (small, (2.35, 4.2), (-1.1, -1.5), -45.0,
         (0.21, 0.0, 0.79, 1.0), (0.31, 0.1, 0.69, 0.9)),
        (small, (2.35, 4.2), (0.45, -4.45), 45.0,
         (0.21, 0.0, 0.79, 1.0), (0.31, 0.1, 0.69, 0.9)),
        (small, (2.35, 2.35), (-0.385, -1.29), 0.0,
         (0.0, 0.0, 1.0, 1.0), (0.31, 0.1, 0.69, 0.9)),
        (small, (2.35, 2.35), (-0.965, -1.29), 0.0,
         (0.0, 0.0, 1.0, 1.0), (0.31, 0.1, 0.69, 0.9)),
    ]
    previous = large
    for source, scale, offset, angle, scissor, vignette in passes:
        pixels, fallback = source.load(), previous.load()
        output = Image.new("RGB", image.size)
        result = output.load()
        radians = angle * 0.0174532925
        cosine, sine = math.cos(radians), math.sin(radians)
        for y in range(height):
            v = (height - y - 0.5) / height
            for x in range(width):
                u = (x + 0.5) / width
                su = (u * cosine - v * sine) * scale[0] + offset[0]
                sv = (v * cosine + u * sine) * scale[1] + offset[1]
                sx = min(width - 1, max(0, math.floor(su * width)))
                sy = height - 1 - min(height - 1, max(0, math.floor(sv * height)))
                background = fallback[x, y]
                color = pixels[sx, sy]
                if su < scissor[0] or sv < scissor[1] or su > scissor[2] or sv > scissor[3]:
                    color = background
                # Preserve sequential GLSL if/mix operations, not a single
                # radial vignette, clamped interpolation, or fitted mask.
                for coordinate, index, outside in ((su, 0, su < vignette[0]),
                        (sv, 1, sv < vignette[1]), (su, 2, su > vignette[2]),
                        (sv, 3, sv > vignette[3])):
                    if outside:
                        weight = (scissor[index] - coordinate) / (scissor[index] - vignette[index])
                        color = tuple(b + (c - b) * weight for b, c in zip(background, color))
                result[x, y] = tuple(round(min(255, max(0, c))) for c in color)
        previous = output
    # Final blit ColorModulate = (1, .8, .8, 1), then RGBA8 storage.
    return previous.point([round(v * red_multiplier) for v in range(256)] + [round(v * 0.8) for v in range(256)] * 2)
