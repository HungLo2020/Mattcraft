"""Lossless bounded RGB decoding of supported X11 drawable dumps.

The XWDFileHeader layout is defined by X11/XWDFile.h. Unused pixel bits are
not an instruction to premultiply or otherwise change drawable RGB values.
"""
import struct
from pathlib import Path
from PIL import Image


def decode_truecolor_rgb(data: bytes) -> Image.Image:
    if not 100 <= len(data) <= 128 * 1024 * 1024:
        raise ValueError("XWD payload outside bounded range")
    fields = struct.unpack_from(">25I", data)
    header_size, version, pixmap_format, depth, width, height, xoffset, byte_order = fields[:8]
    bits_per_pixel, stride, visual, red, green, blue = fields[11:17]
    colors = fields[19]
    if (version != 7 or pixmap_format != 2 or depth not in (24, 32)
            or bits_per_pixel not in (24, 32) or depth > bits_per_pixel
            or visual not in (4, 5) or xoffset != 0
            or byte_order not in (0, 1)
            or (red, green, blue) != (0xFF0000, 0xFF00, 0xFF)):
        raise ValueError("unsupported XWD RGB layout")
    pixel_bytes = bits_per_pixel // 8
    if (not 1 <= width <= 8192 or not 1 <= height <= 8192
            or width * height > 16_777_216 or not width * pixel_bytes <= stride <= width * pixel_bytes + 4096
            or not 100 <= header_size <= 1024 * 1024 or colors > 65536):
        raise ValueError("invalid or unbounded XWD extent/header")
    offset = header_size + colors * 12
    if offset + stride * height != len(data):
        raise ValueError("incomplete or trailing XWD pixel payload")
    mode = ("BGRX" if byte_order == 0 else "XRGB") if bits_per_pixel == 32 else ("BGR" if byte_order == 0 else "RGB")
    tables = [[None] * 256 for _ in range(3)]
    if visual == 5:
        # DirectColor pixels are three independent colour-table indices, not
        # literal RGB. XWD query dumps may leave flags zero (all RGB returned).
        # Require complete, consistent tables exactly representable in RGB8;
        # never guess a missing entry or round higher-precision components.
        for index in range(colors):
            pixel, r, g, b, flags, _ = struct.unpack_from(">IHHHBB", data, header_size + index * 12)
            if flags & ~7:
                raise ValueError("invalid XWD DirectColor component flags")
            for channel, (shift, value) in enumerate(zip((16,8,0), (r,g,b))):
                if flags and not flags & (1 << channel):
                    continue
                slot = (pixel >> shift) & 255
                if value % 257:
                    raise ValueError("XWD DirectColor component is not lossless RGB8")
                previous = tables[channel][slot]
                if previous is not None and previous != value // 257:
                    raise ValueError("conflicting XWD DirectColor entries")
                tables[channel][slot] = value // 257
        if any(value is None for table in tables for value in table):
            raise ValueError("incomplete XWD DirectColor tables")
    image = Image.frombytes("RGB", (width, height), data[offset:], "raw", mode, stride, 1)
    if visual == 5:
        with image:
            return image.point([value for table in tables for value in table])
    return image


def write_png(source: Path, destination: Path) -> None:
    if source.resolve() == destination.resolve() or source.stat().st_size > 128 * 1024 * 1024:
        raise ValueError("invalid XWD conversion paths or size")
    with decode_truecolor_rgb(source.read_bytes()) as image:
        image.save(destination, "PNG")


def main() -> int:
    import argparse
    import sys
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()
    try:
        write_png(args.source, args.destination)
    except (ValueError, OSError) as error:
        print(f"XWD capture conversion rejected: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
