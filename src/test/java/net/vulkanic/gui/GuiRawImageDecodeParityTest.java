package net.vulkanic.gui;

import java.awt.image.BufferedImage;
import java.io.ByteArrayOutputStream;
import javax.imageio.ImageIO;
import net.minecraft.resources.ResourceLocation;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class GuiRawImageDecodeParityTest {
    private static byte[] png(BufferedImage image) throws Exception {
        var out = new ByteArrayOutputStream();
        assertTrue(ImageIO.write(image, "PNG", out));
        return out.toByteArray();
    }

    @Test void grayscalePngRetainsVanillaSampleBytesInsteadOfAwtGammaConversion() throws Exception {
        var image = new BufferedImage(4, 1, BufferedImage.TYPE_BYTE_GRAY);
        int[] samples = {0, 64, 128, 255};
        for (int x = 0; x < samples.length; x++) image.getRaster().setSample(x, 0, 0, samples[x]);
        assertNotEquals(128, image.getRGB(2, 0) & 255, "fixture must expose AWT color conversion");
        var asset = RustGalGuiRawImageAssets.decode(ResourceLocation.withDefaultNamespace("test/gray"), png(image), 4);
        assertNotNull(asset);
        byte[] pixels = asset.pixels();
        for (int x = 0; x < samples.length; x++) {
            for (int c = 0; c < 3; c++) assertEquals(samples[x], pixels[x * 4 + c] & 255);
            assertEquals(255, pixels[x * 4 + 3] & 255);
        }
    }

    @Test void rgbaChannelsAndTransparencyRemainUnmodified() throws Exception {
        var image = new BufferedImage(1, 1, BufferedImage.TYPE_INT_ARGB);
        image.setRGB(0, 0, 0x7f1234ab);
        var asset = RustGalGuiRawImageAssets.decode(ResourceLocation.withDefaultNamespace("test/rgba"), png(image), 1);
        assertNotNull(asset);
        assertArrayEquals(new byte[]{0x12, 0x34, (byte)0xab, 0x7f}, asset.pixels());
    }

    @Test void malformedAndOversizedPngAreRejectedBeforePixelAllocation() throws Exception {
        var id = ResourceLocation.withDefaultNamespace("test/bounds");
        assertNull(RustGalGuiRawImageAssets.decode(id, new byte[0], 4));
        assertNull(RustGalGuiRawImageAssets.decode(id, png(new BufferedImage(3, 2, BufferedImage.TYPE_INT_ARGB)), 4));
    }
}
