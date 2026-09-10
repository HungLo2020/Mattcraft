import sys
from pathlib import Path
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "Common"))
from terrain_clip_reference import decode_clip_row


class Pixels:
    def __init__(self, words):
        self.pixels = {}
        for i, word in enumerate(words):
            self.pixels[(760 + i * 2, 320)] = (word & 255, (word >> 8) & 255, (word >> 16) & 255)
            self.pixels[(761 + i * 2, 320)] = ((word >> 24) & 255, 165, 90)

    def getpixel(self, position):
        return self.pixels[position]


class TerrainClipReferenceTest(unittest.TestCase):
    def test_complete_float_words_preserve_sign_exponent_and_mantissa(self):
        for words in ((0x3F800000, 0xBF800000, 0x80000000, 0),
                      (0x7FC01234, 0x7F800000, 0xFF800000, 0x00000001)):
            self.assertEqual(words, decode_clip_row(Pixels(words), 760, 320))

    def test_unknown_color_output_is_not_misread_as_a_clip_measurement(self):
        image = Pixels((1, 2, 3, 4))
        image.pixels[(763, 320)] = (0, 0, 0)
        with self.assertRaises(ValueError):
            decode_clip_row(image, 760, 320)

    def test_alignment_and_rgb_contract_fail_closed(self):
        image = Pixels((1, 2, 3, 4))
        for x, y in ((764, 320), (-8, 320), (760, -1)):
            with self.assertRaises(ValueError):
                decode_clip_row(image, x, y)
        image.pixels[(760, 320)] = (0, 0, 0, 255)
        with self.assertRaises(ValueError):
            decode_clip_row(image, 760, 320)


if __name__ == "__main__":
    unittest.main()
