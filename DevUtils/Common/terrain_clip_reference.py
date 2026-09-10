"""Decode private GPU clip-coordinate observations; never a color-parity gate."""


def decode_clip_row(image, x: int, y: int) -> tuple[int, int, int, int]:
    """Read four complete float bitwords from an aligned eight-pixel block.

    Even pixels carry low 24 bits; odd pixels carry the high byte and a fixed
    two-byte tag. No exponent, sign, finite value or alpha is assumed.
    Callers must independently establish that the block shares one primitive.
    """
    if x < 0 or y < 0 or x % 8:
        raise ValueError("clip observation requires a nonnegative eight-pixel-aligned origin")
    words = []
    for component in range(4):
        low = image.getpixel((x + component * 2, y))
        high = image.getpixel((x + component * 2 + 1, y))
        if len(low) != 3 or len(high) != 3 or high[1:] != (165, 90):
            raise ValueError("clip observation tag or RGB format mismatch")
        if any(not isinstance(value, int) or not 0 <= value <= 255 for value in (*low, *high)):
            raise ValueError("clip observation contains non-byte channels")
        words.append(low[0] | low[1] << 8 | low[2] << 16 | high[0] << 24)
    return tuple(words)
