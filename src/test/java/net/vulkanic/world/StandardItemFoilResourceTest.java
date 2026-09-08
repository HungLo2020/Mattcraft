package net.vulkanic.world;

import java.io.ByteArrayInputStream;
import java.io.IOException;
import java.lang.reflect.Proxy;
import java.nio.charset.StandardCharsets;
import java.util.Base64;
import java.util.concurrent.atomic.AtomicBoolean;
import net.minecraft.server.packs.PackResources;
import net.minecraft.server.packs.resources.Resource;
import net.minecraft.server.packs.resources.ResourceMetadata;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class StandardItemFoilResourceTest {
    private static final byte[] PNG = Base64.getDecoder().decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLttAAAAABJRU5ErkJggg==");
    private static PackResources pack() {
        return (PackResources) Proxy.newProxyInstance(PackResources.class.getClassLoader(),
            new Class<?>[]{PackResources.class}, (proxy, method, args) -> {
                if (method.getName().equals("packId")) return "selected-pack";
                throw new UnsupportedOperationException(method.getName());
            });
    }

    @Test void copiesSelectedBytesAndEverySamplingCombinationWithoutGpuAccess() throws Exception {
        for (boolean blur : new boolean[]{false,true}) for (boolean clamp : new boolean[]{false,true}) {
            var closed = new AtomicBoolean();
            var resource = new Resource(pack(), () -> new ByteArrayInputStream(PNG) {
                @Override public void close() { closed.set(true); }
            }, () -> ResourceMetadata.fromJsonStream(new ByteArrayInputStream(
                ("{\"texture\":{\"blur\":"+blur+",\"clamp\":"+clamp+"}}").getBytes(StandardCharsets.UTF_8))));
            var asset = RustGalWorldPrimitiveRenderer.copyStandardItemFoilTexture(123,resource);
            assertTrue(closed.get());
            assertArrayEquals(PNG,asset.pngBytes());
            assertEquals(123,asset.textureId());
            assertEquals(blur ? 2 : 1,asset.samplingFilter());
            assertEquals(clamp ? 2 : 1,asset.samplingAddress());
            assertEquals(1,asset.requestedMipLevels());
            assertEquals(0,asset.coordinateOrigin());
        }
    }

    @Test void absentMetadataUsesExplicitResourceDefaults() throws Exception {
        var asset = RustGalWorldPrimitiveRenderer.copyStandardItemFoilTexture(123,
            new Resource(pack(), () -> new ByteArrayInputStream(PNG)));
        assertEquals(1,asset.samplingFilter());
        assertEquals(1,asset.samplingAddress());
        assertEquals(1,asset.requestedMipLevels());
    }

    @Test void brokenSelectedMetadataFailsClosedAndClosesPixels() {
        var closed = new AtomicBoolean();
        var resource = new Resource(pack(), () -> new ByteArrayInputStream(PNG) {
            @Override public void close() { closed.set(true); }
        }, () -> { throw new IOException("selected metadata unreadable"); });
        assertThrows(IOException.class, () -> RustGalWorldPrimitiveRenderer.copyStandardItemFoilTexture(123,resource));
        assertTrue(closed.get());
    }
}
