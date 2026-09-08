package net.vulkanic.bridge;

import java.awt.image.BufferedImage;
import java.io.ByteArrayOutputStream;
import java.util.List;
import javax.imageio.ImageIO;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class WorldMeshTextureMipContractTest {
    @Test
    void nativeDecoderReceivesAndValidatesExplicitResourceMipCount() throws Exception {
        var output = new ByteArrayOutputStream();
        assertTrue(ImageIO.write(new BufferedImage(8, 8, BufferedImage.TYPE_INT_ARGB), "PNG", output));
        var texture = new VulkanicGalBridge.WorldMeshTextureAssetRecord(17, output.toByteArray());
        try (var bridge = VulkanicGalBridge.create("rust-vulkan")) {
            bridge.updateWorldMeshAssets(1, List.of(), List.of(texture.withMipLevels(1)), List.of());
            bridge.updateWorldMeshAssets(2, List.of(), List.of(texture.withMipLevels(4)), List.of());
            // Five levels cannot exist in an 8x8 resource. This must fail in
            // Rust; a Java encoder that drops the new field would accept it.
            var error = assertThrows(IllegalStateException.class, () ->
                bridge.updateWorldMeshAssets(3, List.of(), List.of(texture.withMipLevels(5)), List.of()));
            assertTrue(error.getMessage().contains("mip count"));
        }
    }
}
