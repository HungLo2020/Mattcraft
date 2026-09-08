package net.vulkanic.world;

import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.awt.image.BufferedImage;
import javax.imageio.ImageIO;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.server.packs.PackResources;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.resources.FallbackResourceManager;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;
import static org.mockito.Mockito.*;

class SemanticTexturePackPrecedenceTest {
    private static final ResourceLocation TEXTURE = ResourceLocation.withDefaultNamespace("textures/item/apple.png");

    @BeforeAll static void bootstrap() {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
    }

    private static PackResources pack(String name, int size, int color) throws Exception {
        var image = new BufferedImage(size, size, BufferedImage.TYPE_INT_ARGB);
        for (int y = 0; y < size; y++) for (int x = 0; x < size; x++) image.setRGB(x, y, color);
        var encoded = new ByteArrayOutputStream();
        assertTrue(ImageIO.write(image, "png", encoded));
        byte[] bytes = encoded.toByteArray();
        var pack = mock(PackResources.class);
        when(pack.packId()).thenReturn(name);
        when(pack.getResource(PackType.CLIENT_RESOURCES, TEXTURE))
            .thenReturn(() -> new ByteArrayInputStream(bytes));
        return pack;
    }

    @Test void copiedTextureMatchesActualManagerWinnerAcrossPriorityAndReloadChanges() throws Exception {
        var vanilla = pack("vanilla", 16, 0xff123456);
        var a = pack("A", 16, 0xffee2543);
        var b = pack("B", 32, 0xff23d462);
        // Real resource manager, not an invented stack-order mock. Removing A
        // models resource reload; reversing A/B models user pack precedence.
        for (var packs : java.util.List.of(java.util.List.of(vanilla, b, a),
                java.util.List.of(vanilla, b), java.util.List.of(vanilla, a, b))) {
            var manager = new FallbackResourceManager(PackType.CLIENT_RESOURCES, "minecraft");
            packs.forEach(manager::push);
            var stack = manager.getResourceStack(TEXTURE);
            assertEquals("vanilla", stack.getFirst().sourcePackId());
            assertEquals(packs.getLast().packId(), stack.getLast().sourcePackId());
            byte[] expected;
            try (var stream = manager.getResource(TEXTURE).orElseThrow().open()) {
                expected = stream.readAllBytes();
            }
            byte[] actual = RustGalWorldPrimitiveRenderer.readTexturePayloadForResource(TEXTURE, stack);
            assertArrayEquals(expected, actual);
            var decoded = ImageIO.read(new ByteArrayInputStream(actual));
            boolean winnerA = packs.getLast() == a;
            assertEquals(winnerA ? 16 : 32, decoded.getWidth());
            assertEquals(winnerA ? 0xffee2543 : 0xff23d462, decoded.getRGB(0, 0));
        }
    }
}
