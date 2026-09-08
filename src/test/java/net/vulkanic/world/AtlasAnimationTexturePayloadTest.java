package net.vulkanic.world;

import java.io.ByteArrayInputStream;
import java.util.List;
import javax.imageio.ImageIO;
import net.minecraft.client.renderer.texture.SemanticAtlasAnimationSource;
import net.minecraft.client.renderer.texture.SpriteContents;
import net.minecraft.client.renderer.texture.TextureAtlas;
import net.minecraft.resources.ResourceLocation;
import net.vulkanic.bridge.VulkanicGalBridge;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class AtlasAnimationTexturePayloadTest {
    @org.junit.jupiter.api.BeforeAll static void bootstrap() {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
    }
    private static final ResourceLocation ATLAS = TextureAtlas.LOCATION_PARTICLES;
    private static final int ID = RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_PARTICLE_ATLAS;
    private static AtlasAnimationResource resource() {
        var animation = new SpriteContents.SemanticAnimationSource(2, 2, 1, true,
            List.of(new SpriteContents.SemanticAnimationFrame(0, 2), new SpriteContents.SemanticAnimationFrame(1, 2)),
            List.of(new SpriteContents.SemanticAnimationMip(2, 4, new byte[32]),
                    new SpriteContents.SemanticAnimationMip(1, 2, new byte[8])));
        return new AtlasAnimationResource(ATLAS, ID, new SemanticAtlasAnimationSource(7, 2, 2, 2,
            List.of(new SemanticAtlasAnimationSource.Sprite(1,
                ResourceLocation.withDefaultNamespace("particle/test"), 0, 0, animation))));
    }
    private static TextureAtlas.SemanticRawSnapshot snapshot() {
        byte[] rgba = {1,2,3,4, 11,12,13,14, 21,22,23,24, 31,32,33,34};
        return new TextureAtlas.SemanticRawSnapshot(ATLAS, 7, 2, 2, rgba,
            List.of(rgba, new byte[]{41,42,43,44}));
    }

    @Test void exactRgbaRowsAndSuppliedMipAllocationReachRustWithoutJavaGpuObjects() throws Exception {
        try (var resource = resource()) {
            var payload = AtlasTexturePayload.copy(resource, snapshot());
            assertEquals(ID, payload.textureId());
            assertEquals(2, payload.requestedMipLevels());
            var base = ImageIO.read(new ByteArrayInputStream(payload.pngBytes()));
            assertEquals(0x04010203, base.getRGB(0, 0));
            assertEquals(0x0e0b0c0d, base.getRGB(1, 0));
            assertEquals(0x18151617, base.getRGB(0, 1));
            assertEquals(0x221f2021, base.getRGB(1, 1));
            var mip = ImageIO.read(new ByteArrayInputStream(payload.mipPngBytes().getFirst()));
            assertEquals(0x2c292a2b, mip.getRGB(0, 0));
            var publication = new AtlasAnimationPublication(payload, resource);
            try (var bridge = VulkanicGalBridge.create("rust-vulkan")) {
                bridge.updateWorldMeshAssets(11, List.of(), List.of(payload), List.of());
                publication.textureAccepted(11, payload);
                publication.flush(bridge::stageAtlasAnimationAssets);
                resource.recordUse(ATLAS, resource.source().sprites().getFirst().name());
                resource.enqueueNextTick(true);
                assertTrue(publication.drainTicks(bridge));
            }
        }
    }

    @Test void mismatchedIncarnationsAndIncompleteChainsRejectWithoutPublication() throws Exception {
        try (var resource = resource()) {
            var snapshot = snapshot();
            assertThrows(IllegalArgumentException.class, () -> AtlasTexturePayload.copy(resource, null));
            for (var bad : List.of(
                new TextureAtlas.SemanticRawSnapshot(TextureAtlas.LOCATION_BLOCKS, 7, 2, 2, snapshot.pixels(), snapshot.mipPixels()),
                new TextureAtlas.SemanticRawSnapshot(ATLAS, 8, 2, 2, snapshot.pixels(), snapshot.mipPixels()),
                new TextureAtlas.SemanticRawSnapshot(ATLAS, 7, 1, 2, snapshot.pixels(), snapshot.mipPixels()),
                new TextureAtlas.SemanticRawSnapshot(ATLAS, 7, 2, 2, snapshot.pixels(), List.of(snapshot.pixels())),
                new TextureAtlas.SemanticRawSnapshot(ATLAS, 7, 2, 2, new byte[16], snapshot.mipPixels()),
                new TextureAtlas.SemanticRawSnapshot(ATLAS, 7, 2, 2, snapshot.pixels(), List.of(snapshot.pixels(), new byte[8])))) {
                assertThrows(IllegalArgumentException.class, () -> AtlasTexturePayload.copy(resource, bad));
            }
            resource.close();
            assertThrows(IllegalStateException.class, () -> AtlasTexturePayload.copy(resource, snapshot));
        }
    }

    @Test void staticAtlasPreservesSuppliedMipsAndRejectsAnImplicitOrIncompleteChain() throws Exception {
        var snapshot = snapshot();
        var payload = AtlasTexturePayload.copy(ID, ATLAS, 7, 2, snapshot);
        assertEquals(2, payload.requestedMipLevels());
        assertEquals(0x2c292a2b, ImageIO.read(new ByteArrayInputStream(payload.mipPngBytes().getFirst())).getRGB(0, 0));
        var changed = new TextureAtlas.SemanticRawSnapshot(ATLAS, 8, 2, 2, snapshot.pixels(),
            List.of(snapshot.pixels(), new byte[]{51,52,53,54}));
        var replacement = AtlasTexturePayload.copy(ID, ATLAS, 8, 2, changed);
        assertArrayEquals(payload.pngBytes(), replacement.pngBytes(), "mip zero deliberately unchanged");
        assertFalse(java.util.Arrays.equals(payload.mipPngBytes().getFirst(), replacement.mipPngBytes().getFirst()));
        var textures = new java.util.LinkedHashMap<Integer, VulkanicGalBridge.WorldMeshTextureAssetRecord>();
        var dirty = new java.util.LinkedHashSet<Integer>();
        assertTrue(RustGalWorldPrimitiveRenderer.registerChangedTexture(textures, dirty, payload));
        dirty.clear();
        assertTrue(RustGalWorldPrimitiveRenderer.registerChangedTexture(textures, dirty, replacement));
        assertTrue(dirty.contains(ID), "higher-mip changes must enter the actual native publication queue");
        dirty.clear();
        assertFalse(RustGalWorldPrimitiveRenderer.registerChangedTexture(textures, dirty, replacement));
        assertTrue(dirty.isEmpty(), "an identical complete payload does not trigger another native upload");
        var baseOnly = new TextureAtlas.SemanticRawSnapshot(ATLAS, 9, 2, 2, snapshot.pixels(), List.of(snapshot.pixels()));
        assertEquals(1, AtlasTexturePayload.copy(ID, ATLAS, 9, 1, baseOnly).requestedMipLevels());
        assertThrows(IllegalArgumentException.class, () -> AtlasTexturePayload.copy(ID, ATLAS, 9, 2, baseOnly));
        assertThrows(IllegalArgumentException.class, () -> AtlasTexturePayload.copy(ID, ATLAS, 7, 0, snapshot));
        assertThrows(IllegalArgumentException.class, () -> AtlasTexturePayload.copy(ID, TextureAtlas.LOCATION_BLOCKS, 7, 2, snapshot));
        assertThrows(IllegalArgumentException.class, () -> AtlasTexturePayload.copy(ID, ATLAS, 8, 2, snapshot));
        try (var bridge = VulkanicGalBridge.create("rust-vulkan")) {
            bridge.updateWorldMeshAssets(21, List.of(), List.of(payload), List.of());
            bridge.updateWorldMeshAssets(22, List.of(), List.of(replacement), List.of());
            bridge.updateWorldMeshAssets(23, List.of(), List.of(AtlasTexturePayload.copy(ID, ATLAS, 9, 1, baseOnly)), List.of());
        }
    }
}
