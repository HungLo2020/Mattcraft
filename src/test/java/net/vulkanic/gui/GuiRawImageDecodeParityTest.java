package net.vulkanic.gui;

import java.awt.image.BufferedImage;
import java.io.ByteArrayOutputStream;
import javax.imageio.ImageIO;
import net.minecraft.resources.ResourceLocation;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class GuiRawImageDecodeParityTest {
    @Test void selectedResourceSamplingIsCopiedWithPixelsAndReloadMetadata() throws Exception {
        var bytes = png(new BufferedImage(1, 1, BufferedImage.TYPE_INT_ARGB));
        var pack = (net.minecraft.server.packs.PackResources) java.lang.reflect.Proxy.newProxyInstance(
            getClass().getClassLoader(), new Class<?>[]{net.minecraft.server.packs.PackResources.class},
            (proxy, method, args) -> { if (method.getName().equals("packId")) return "fixture";
                throw new UnsupportedOperationException(method.getName()); });
        var id = ResourceLocation.withDefaultNamespace("test/sampler");
        for (boolean blur : new boolean[]{false,true}) for (boolean clamp : new boolean[]{false,true}) {
            var resource = new net.minecraft.server.packs.resources.Resource(pack,
                () -> new java.io.ByteArrayInputStream(bytes),
                () -> net.minecraft.server.packs.resources.ResourceMetadata.fromJsonStream(new java.io.ByteArrayInputStream(
                    ("{\"texture\":{\"blur\":"+blur+",\"clamp\":"+clamp+"}}").getBytes(java.nio.charset.StandardCharsets.UTF_8))));
            var asset = RustGalGuiRawImageAssets.decode(id, resource, 1);
            assertNotNull(asset);
            assertEquals(blur ? 2 : 1, asset.samplingFilter());
            assertEquals(clamp ? 2 : 1, asset.samplingAddress());
            assertArrayEquals(new byte[4], asset.pixels());
        }
        var defaults = RustGalGuiRawImageAssets.decode(id,
            new net.minecraft.server.packs.resources.Resource(pack, () -> new java.io.ByteArrayInputStream(bytes)), 1);
        assertEquals(1, defaults.samplingFilter());
        assertEquals(1, defaults.samplingAddress());
        var broken = new net.minecraft.server.packs.resources.Resource(pack, () -> new java.io.ByteArrayInputStream(bytes),
            () -> { throw new java.io.IOException("bad selected metadata"); });
        assertNull(RustGalGuiRawImageAssets.decode(id, broken, 1));
    }

    @Test void transportRejectsPartialOrUnknownSamplerAndCoordinatorRetainsMetadataOnlyChanges() throws Exception {
        for (int filter=0; filter<=3; filter++) for (int address=0; address<=3; address++) {
            final int f=filter, a=address;
            boolean valid = (f==0 && a==0) || (f>=1 && f<=2 && a>=1 && a<=2);
            if (valid) assertDoesNotThrow(() -> new net.vulkanic.bridge.VulkanicGalBridge.GuiRawImageAssetRecord(1,2,1,1,new byte[4],f,a));
            else assertThrows(IllegalArgumentException.class, () -> new net.vulkanic.bridge.VulkanicGalBridge.GuiRawImageAssetRecord(1,2,1,1,new byte[4],f,a));
        }
        var source = java.nio.file.Files.readString(java.nio.file.Path.of("src/main/java/net/vulkanic/gui/RustGalFrameCoordinator.java"));
        assertTrue(source.contains("previous.samplingFilter() == asset.samplingFilter()"));
        assertTrue(source.contains("previous.samplingAddress() == asset.samplingAddress()"));
    }

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
