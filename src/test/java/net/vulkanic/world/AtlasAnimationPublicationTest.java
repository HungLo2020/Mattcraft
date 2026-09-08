package net.vulkanic.world;

import java.util.List;
import java.util.concurrent.atomic.AtomicInteger;
import net.minecraft.client.renderer.texture.SemanticAtlasAnimationSource;
import net.minecraft.client.renderer.texture.SpriteContents;
import net.minecraft.resources.ResourceLocation;
import net.vulkanic.bridge.VulkanicGalBridge;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class AtlasAnimationPublicationTest {
    private static final int ATLAS = 0x54a17a1a;
    private static AtlasAnimationResource resource(SemanticAtlasAnimationSource source) {
        return new AtlasAnimationResource(net.minecraft.client.renderer.texture.TextureAtlas.LOCATION_BLOCKS, ATLAS, source);
    }

    private static SemanticAtlasAnimationSource source() {
        return new SemanticAtlasAnimationSource(77, 2, 1, 1, List.of(
            new SemanticAtlasAnimationSource.Sprite(1, ResourceLocation.withDefaultNamespace("audit/sprite"), 0, 0,
                new SpriteContents.SemanticAnimationSource(1, 1, 2, true,
                    List.of(new SpriteContents.SemanticAnimationFrame(0, 2), new SpriteContents.SemanticAnimationFrame(1, 3)),
                    List.of(new SpriteContents.SemanticAnimationMip(2, 1, new byte[]{10,20,30,7,110,120,(byte)130,99}))))));
    }

    @Test
    void stagingFailureRetainsOnlyAcceptedGenerationForRetry() {
        var texture = new VulkanicGalBridge.WorldMeshTextureAssetRecord(ATLAS, new byte[]{1}, List.of());
        var source = source();
        var publication = new AtlasAnimationPublication(texture, resource(source));
        AtomicInteger calls = new AtomicInteger();
        AtlasAnimationPublication.Stage stage = (id, generation, tick, copied) -> {
            assertEquals(ATLAS, id);
            assertEquals(12, generation);
            assertEquals(0, tick);
            assertSame(source, copied);
            if (calls.incrementAndGet() == 1) throw new IllegalStateException("injected staging failure");
            return null;
        };
        assertNull(publication.flush(stage));
        assertEquals(0, calls.get());
        assertThrows(IllegalArgumentException.class, () -> publication.textureAccepted(12,
            new VulkanicGalBridge.WorldMeshTextureAssetRecord(ATLAS, new byte[]{2}, List.of())));
        publication.textureAccepted(12, texture);
        assertThrows(IllegalStateException.class, () -> publication.flush(stage));
        assertTrue(publication.pending());
        publication.flush(stage);
        assertFalse(publication.pending());
        publication.textureAccepted(12, texture);
        publication.flush(stage);
        assertEquals(2, calls.get(), "accepted declarations must not be submitted twice");
        assertThrows(IllegalArgumentException.class, () -> publication.textureAccepted(11, texture));
    }

    @Test
    void queuedEventsRemainEpochBoundAcrossDuplicateReceiptsFailuresAndReplacement() {
        var texture = new VulkanicGalBridge.WorldMeshTextureAssetRecord(ATLAS, new byte[]{1}, List.of());
        var resource = resource(source());
        var publication = new AtlasAnimationPublication(texture, resource);
        var atlas = net.minecraft.client.renderer.texture.TextureAtlas.LOCATION_BLOCKS;
        var sprite = source().sprites().getFirst().name();
        publication.recordSpriteUse(atlas, sprite);
        publication.enqueueTick(1, true);
        publication.textureAccepted(12, texture);
        assertThrows(IllegalStateException.class, () -> publication.flush((id, gen, tick, data) -> {
            throw new IllegalStateException("injected metadata failure");
        }));
        assertThrows(IllegalStateException.class, () -> publication.drainTicks((id, gen, tick, ids, visible) -> {
            fail("unstaged events must not enter the backend");
            return true;
        }));
        publication.flush((id, gen, tick, data) -> null);
        assertFalse(publication.drainTicks((id, gen, tick, ids, visible) -> false));
        publication.textureAccepted(12, texture);
        assertNull(publication.flush((id, gen, tick, data) -> {
            fail("a duplicate receipt must not reset queued events");
            return null;
        }));
        var delivered = new AtomicInteger();
        assertTrue(publication.drainTicks((id, gen, tick, ids, visible) -> {
            assertEquals(ATLAS, id);
            assertEquals(12, gen);
            assertEquals(1, tick);
            assertArrayEquals(new int[]{1}, ids, "failed staging/enqueue must preserve recorded uses");
            assertTrue(visible);
            delivered.incrementAndGet();
            return true;
        }));
        assertEquals(1, delivered.get());
        publication.enqueueTick(2, false);
        assertThrows(IllegalArgumentException.class, () -> publication.textureAccepted(13, texture),
            "replacing native storage must not silently restart this resource's clock");
        resource.close();
        assertThrows(IllegalStateException.class, () -> publication.drainTicks((id, gen, tick, ids, visible) -> {
            fail("old events cannot enter a replaced native image");
            return true;
        }));
        var replacement = new AtlasAnimationPublication(texture, resource(source()));
        replacement.enqueueTick(1, false);
        replacement.textureAccepted(13, texture);
        assertThrows(IllegalStateException.class, () -> replacement.flush((id, gen, tick, data) -> {
            throw new IllegalStateException("replacement staging failed");
        }));
        replacement.flush((id, gen, tick, data) -> null);
        assertTrue(replacement.drainTicks((id, gen, tick, ids, visible) -> {
            assertEquals(13, gen);
            assertEquals(1, tick, "replacement must not deliver tick 2 from the old epoch");
            assertArrayEquals(new int[0], ids);
            assertFalse(visible);
            delivered.incrementAndGet();
            return true;
        }));
        assertEquals(2, delivered.get());
    }

    @Test
    void acceptedTextureReplacementRestagesThroughActualRustContext() throws Exception {
        var png = new java.io.ByteArrayOutputStream();
        assertTrue(javax.imageio.ImageIO.write(new java.awt.image.BufferedImage(2, 1,
            java.awt.image.BufferedImage.TYPE_INT_ARGB), "PNG", png));
        var texture = new VulkanicGalBridge.WorldMeshTextureAssetRecord(ATLAS, png.toByteArray(), List.of());
        var resource = resource(source());
        resource.recordUse(net.minecraft.client.renderer.texture.TextureAtlas.LOCATION_BLOCKS,
            source().sprites().getFirst().name());
        resource.enqueueNextTick(true);
        var publication = new AtlasAnimationPublication(texture, resource);
        try (var bridge = VulkanicGalBridge.create("rust-vulkan")) {
            var before = bridge.updateWorldMeshAssets(1, List.of(), List.of(texture), List.of());
            publication.textureAccepted(1, texture);
            var staged = publication.flush(bridge::stageAtlasAnimationAssets);
            assertEquals(before.ffiCalls() + 1, staged.ffiCalls());
            assertEquals(before.submissionId(), staged.submissionId());
            assertNull(publication.flush(bridge::stageAtlasAnimationAssets));
            publication.enqueueTick(2, true);
            assertTrue(publication.drainTicks(bridge));
            var duplicate = bridge.tickAtlasAnimation(ATLAS, 1, 2, new int[0], true);
            assertTrue(duplicate.accepted());
            assertTrue(duplicate.status().submissionId() > staged.submissionId(),
                "queued visible interpolation must submit actual Rust Vulkan work");
            // A resource pump needs neither a frame target nor a presenter.
            // Keep ticking beyond the entire FIFO capacity without drawing.
            for (int tick = 3; tick <= 153; tick++) {
                resource.enqueueNextTick(false);
                assertTrue(publication.drainTicks(bridge));
                assertEquals(0, publication.pendingTickCount());
            }
            publication.enqueueTick(154, true);
            long beforeReplacement = bridge.tickAtlasAnimation(ATLAS, 1, 153, new int[0], false).status().submissionId();
            bridge.updateWorldMeshAssets(2, List.of(), List.of(texture), List.of());
            var replacementPublication = new AtlasAnimationPublication(texture, resource(source()));
            replacementPublication.enqueueTick(1, false);
            replacementPublication.textureAccepted(2, texture);
            assertNotNull(replacementPublication.flush(bridge::stageAtlasAnimationAssets));
            assertFalse(replacementPublication.pending());
            assertTrue(replacementPublication.drainTicks(bridge));
            var replacement = bridge.tickAtlasAnimation(ATLAS, 2, 1, new int[0], false);
            assertTrue(replacement.accepted());
            assertTrue(replacement.status().submissionId() > beforeReplacement);
        }
    }
}
