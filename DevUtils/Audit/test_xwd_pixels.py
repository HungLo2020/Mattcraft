import struct
import unittest
import tempfile
from pathlib import Path
from PIL import Image
from DevUtils.Common.xwd_pixels import decode_truecolor_rgb, write_png


def fixture(order=0, unused=186):
    fields = [101, 7, 2, 32, 2, 1, 0, order, 32, order, 32, 32, 12,
              4, 0xff0000, 0xff00, 0xff, 8, 256, 0, 2, 1, 0, 0, 0]
    pixels = (bytes([17, 47, 48, unused, 255, 128, 0, unused]) if order == 0
              else bytes([unused, 48, 47, 17, unused, 0, 128, 255]))
    return struct.pack(">25I", *fields) + b"\0" + pixels + b"PAD!"


class XwdPixelsTests(unittest.TestCase):
    def test_directcolor_rejects_missing_conflicting_or_invalid_component_tables(self):
        def direct_dump(last_pixel, last_red, last_flags=0):
            data = bytearray(fixture())
            struct.pack_into(">I",data,13*4,5)
            struct.pack_into(">I",data,19*4,256)
            table = b"".join(struct.pack(">IHHHBB",i*0x010101,i*257,i*257,i*257,0,0) for i in range(255))
            table += struct.pack(">IHHHBB",last_pixel,last_red,254*257,254*257,last_flags,0)
            data[101:101] = table
            return data
        with self.assertRaisesRegex(ValueError,"incomplete.*tables"):
            decode_truecolor_rgb(direct_dump(0xfefefe,254*257))
        with self.assertRaisesRegex(ValueError,"conflicting"):
            decode_truecolor_rgb(direct_dump(0xfefefe,255*257))
        with self.assertRaisesRegex(ValueError,"component flags"):
            decode_truecolor_rgb(direct_dump(0xffffff,255*257,128))
    def test_packed_directcolor_decodes_padding_byte_order_and_independent_tables(self):
        for order in (0,1):
            fields = [101,7,2,24,2,1,0,order,32,order,8,24,8,5,
                      0xff0000,0xff00,0xff,8,256,256,2,1,0,0,0]
            # Non-identity tables prove this cannot treat indices as RGB.
            colors = b"".join(struct.pack(">IHHHBB", i*0x010101,
                (255-i)*257, i*257, (i//2)*257, 0, 0) for i in range(256))
            pixels = bytes((17,47,48,255,128,0) if order == 0 else (48,47,17,0,128,255))
            data = struct.pack(">25I",*fields)+b"\0"+colors+pixels+b"XY"
            with decode_truecolor_rgb(data) as image:
                self.assertEqual([(207,47,8),(255,128,127)], list(image.getdata()))
            wide = fields.copy(); wide[11] = 32; wide[12] = 10
            wide_pixels = bytes((17,47,48,186,255,128,0,186) if order == 0
                                else (186,48,47,17,186,0,128,255))
            with decode_truecolor_rgb(struct.pack(">25I",*wide)+b"\0"+colors+wide_pixels+b"XY") as image:
                self.assertEqual([(207,47,8),(255,128,127)],list(image.getdata()))
            for index, value in ((19,255), (12,5)):
                invalid = bytearray(data); struct.pack_into(">I",invalid,index*4,value)
                with self.assertRaises(ValueError): decode_truecolor_rgb(invalid)
            invalid = bytearray(data); struct.pack_into(">H",invalid,101+4,65534)
            with self.assertRaisesRegex(ValueError,"lossless RGB8"): decode_truecolor_rgb(invalid)

    def test_packed_truecolor_does_not_apply_query_colormap(self):
        fields = [101,7,2,24,2,1,0,0,32,0,8,24,8,4,0xff0000,0xff00,0xff,8,256,0,2,1,0,0,0]
        data = struct.pack(">25I",*fields)+b"\0"+bytes((17,47,48,255,128,0))+b"XY"
        with decode_truecolor_rgb(data) as image:
            self.assertEqual([(48,47,17),(0,128,255)],list(image.getdata()))
    def test_png_roundtrip_preserves_drawable_channels_without_colormap_quantization(self):
        data = bytearray(fixture())
        struct.pack_into(">I", data, 17 * 4, 11)  # observed X server color precision
        struct.pack_into(">I", data, 19 * 4, 1)
        # TrueColor's queried color table is not pixel data. Do not substitute
        # its lower-precision components for the drawable's explicit RGB bits.
        data[101:101] = struct.pack(">IHHHBB", 0x303030, 12325, 12325, 12325, 7, 0)
        with tempfile.TemporaryDirectory() as temp:
            source, output = Path(temp) / "source.xwd", Path(temp) / "result.png"
            source.write_bytes(data)
            write_png(source, output)
            with Image.open(output) as image:
                self.assertEqual("RGB", image.mode)
                self.assertEqual((48, 47, 17), image.getpixel((0, 0)))
                self.assertEqual((0, 128, 255), image.getpixel((1, 0)))
            self.assertEqual(bytes(data), source.read_bytes())

    def test_rgb_values_are_exact_for_both_byte_orders_and_all_unused_byte_values(self):
        for order in (0, 1):
            for unused in range(256):
                image = decode_truecolor_rgb(fixture(order, unused))
                self.assertEqual("RGB", image.mode)
                self.assertEqual((48, 47, 17), image.getpixel((0, 0)))
                self.assertEqual((0, 128, 255), image.getpixel((1, 0)))

    def test_rejects_unknown_layouts_and_incomplete_or_unbounded_payloads(self):
        data = fixture()
        for index, value in ((1, 6), (2, 1), (3, 16), (4, 0), (5, 10000), (6, 1),
                             (7, 2), (11, 16), (12, 7), (13, 6), (14, 0xff), (19, 65537)):
            changed = bytearray(data)
            struct.pack_into(">I", changed, index * 4, value)
            with self.assertRaises(ValueError):
                decode_truecolor_rgb(bytes(changed))
        for changed in (data[:99], data[:-1], data + b"extra"):
            with self.assertRaises(ValueError):
                decode_truecolor_rgb(changed)


if __name__ == "__main__":
    unittest.main()
