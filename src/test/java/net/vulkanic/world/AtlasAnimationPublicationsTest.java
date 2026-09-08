package net.vulkanic.world;

import java.util.ArrayList;
import java.util.List;
import net.minecraft.client.renderer.texture.SemanticAtlasAnimationSource;
import net.minecraft.resources.ResourceLocation;
import net.vulkanic.bridge.VulkanicGalBridge;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class AtlasAnimationPublicationsTest {
    private static VulkanicGalBridge.WorldMeshTextureAssetRecord texture(int id, int color) {
        try {
            var image = new java.awt.image.BufferedImage(1, 1, java.awt.image.BufferedImage.TYPE_INT_ARGB);
            image.setRGB(0, 0, color);
            var output = new java.io.ByteArrayOutputStream();
            assertTrue(javax.imageio.ImageIO.write(image, "PNG", output));
            return new VulkanicGalBridge.WorldMeshTextureAssetRecord(id, output.toByteArray(), List.of());
        } catch (java.io.IOException error) { throw new AssertionError(error); }
    }

    private static AtlasAnimationPublication publication(int id) {
        var source = new SemanticAtlasAnimationSource(77, 1, 1, 1, List.of());
        return publication(id, source);
    }

    private static AtlasAnimationPublication publication(int id, SemanticAtlasAnimationSource source) {
        return new AtlasAnimationPublication(texture(id, 0), new AtlasAnimationResource(
            ResourceLocation.withDefaultNamespace("atlas/" + id), id, source));
    }

    @Test
    void sourceBudgetsApplyAcrossAtlasesAndReplacementReleasesOnlyItsOwnBudget() {
        var mip = new net.minecraft.client.renderer.texture.SpriteContents.SemanticAnimationMip(
            1024, 1024, new byte[4 * 1024 * 1024]);
        var animation = new net.minecraft.client.renderer.texture.SpriteContents.SemanticAnimationSource(
            1, 1, 1024, false,
            List.of(new net.minecraft.client.renderer.texture.SpriteContents.SemanticAnimationFrame(0, 1)),
            List.of(mip));
        var sprites = new ArrayList<SemanticAtlasAnimationSource.Sprite>();
        for (int i = 0; i < 24; i++) {
            sprites.add(new SemanticAtlasAnimationSource.Sprite(i + 1,
                ResourceLocation.withDefaultNamespace("sprite/" + i), i, 0, animation));
        }
        // Each immutable sheet is charged per declaration even when the test
        // reuses one Java value. No need to allocate 96 MiB to test accounting.
        var full = new SemanticAtlasAnimationSource(77, 24, 1, 1, sprites);
        var extra = new SemanticAtlasAnimationSource(77, 24, 1, 1, List.of(sprites.getFirst()));
        var registry = new AtlasAnimationPublications();
        registry.register(publication(101, full), () -> {});
        assertThrows(IllegalStateException.class, () -> registry.register(publication(202, extra),
            () -> fail("aggregate byte limit must reject before publication")));
        assertEquals(1, registry.size());
        registry.register(publication(101), () -> {});
        registry.register(publication(202, extra), () -> {});
        assertEquals(2, registry.size());
    }

    @Test
    void retriesStayOnTheRejectedAtlasBeforeNewEarlierAtlasEvents() {
        for (boolean throwFailure : List.of(false, true)) {
            var registry = new AtlasAnimationPublications();
            var a = publication(101);
            var b = publication(202);
            registry.register(a, () -> {});
            registry.register(b, () -> {});
            registry.textureAccepted(1, texture(101, 0));
            registry.textureAccepted(1, texture(202, 0));
            a.enqueueTick(1, false);
            b.enqueueTick(1, false);
            var calls = new ArrayList<String>();
            AtlasAnimationTickDelivery.Submit failB = (id, gen, tick, visible, onlyVisible) -> {
                calls.add(id + ":" + tick);
                if (id == 202 && throwFailure) throw new IllegalStateException("pending native upload");
                return id != 202;
            };
            if (throwFailure) assertThrows(IllegalStateException.class, () -> registry.drain((i,g,t,s) -> null, failB));
            else assertFalse(registry.drain((i,g,t,s) -> null, failB));
            assertEquals(List.of("101:1", "202:1"), calls);
            a.enqueueTick(2, false);
            calls.clear();
            assertTrue(registry.drain((i,g,t,s) -> { fail("must not restage"); return null; },
                (id, gen, tick, visible, onlyVisible) -> {
                    calls.add(id + ":" + tick);
                    assertEquals(1, gen);
                    return true;
                }));
            assertEquals(List.of("202:1", "101:2"), calls);
        }
    }

    @Test
    void replacementCannotReleasePendingRetryUntilNewNativeIncarnationIsAccepted() {
        var registry = new AtlasAnimationPublications();
        var a = publication(101);
        var b = publication(202);
        registry.register(a, () -> {});
        registry.register(b, () -> {});
        registry.textureAccepted(1, texture(101, 0));
        registry.textureAccepted(1, texture(202, 0));
        b.enqueueTick(1, false);
        assertFalse(registry.drain((i,g,t,s) -> null, (i,g,t,v,o) -> false));
        a.enqueueTick(1, false);
        registry.register(publication(202), () -> {});
        assertFalse(registry.drain((i,g,t,s) -> null, (i,g,t,v,o) -> {
            fail("old native retry is still pending"); return true;
        }));
        registry.textureAccepted(2, texture(202, 0));
        assertTrue(registry.drain((i,g,t,s) -> null, (i,g,t,v,o) -> {
            assertEquals(101, i); assertEquals(1, g); return true;
        }));
        assertEquals(2, registry.size());
        registry.textureAccepted(3, texture(202, 0xff123456));
        assertEquals(1, registry.size(), "unrelated nonanimated replacement retires only its declarations");
        registry.clear();
        assertEquals(0, registry.size());
    }

    @Test
    void rejectedRegistrationPreservesOldPublicationAndDoesNotPublishTexture() {
        var registry = new AtlasAnimationPublications();
        var first = publication(1);
        registry.register(first, () -> {});
        for (int id = 2; id <= AtlasAnimationPublications.MAX_ATLASES; id++) {
            registry.register(publication(id), () -> {});
        }
        assertThrows(IllegalStateException.class, () -> registry.register(publication(65),
            () -> fail("capacity rejection must precede texture registration")));
        assertThrows(IllegalStateException.class, () -> registry.register(publication(1),
            () -> { throw new IllegalStateException("texture budget rejection"); }));
        for (int id = 1; id <= AtlasAnimationPublications.MAX_ATLASES; id++) {
            registry.textureAccepted(1, texture(id, 0));
        }
        first.enqueueTick(1, false);
        assertTrue(registry.drain((i,g,t,s) -> null, (i,g,t,v,o) -> {
            assertEquals(1, i); assertEquals(1, t); return true;
        }));
        assertEquals(64, registry.size());
    }

    @Test
    void distinctAcceptedGenerationsReachTheActualRustContext() {
        var registry = new AtlasAnimationPublications();
        var a = publication(101);
        var b = publication(202);
        registry.register(a, () -> {});
        registry.register(b, () -> {});
        try (var bridge = VulkanicGalBridge.create("rust-vulkan")) {
            bridge.updateWorldMeshAssets(1, List.of(), List.of(texture(101, 0)), List.of());
            registry.textureAccepted(1, texture(101, 0));
            bridge.updateWorldMeshAssets(2, List.of(), List.of(texture(202, 0)), List.of());
            registry.textureAccepted(2, texture(202, 0));
            a.enqueueTick(1, false);
            b.enqueueTick(1, false);
            var calls = new ArrayList<Integer>();
            assertTrue(registry.drain(bridge::stageAtlasAnimationAssets, (id, gen, tick, visible, onlyVisible) -> {
                assertEquals(id == 101 ? 1 : 2, gen);
                calls.add(id);
                return bridge.tickAtlasAnimation(id, gen, tick, visible, onlyVisible).accepted();
            }));
            assertEquals(List.of(101, 202), calls);
        }
    }

    @Test
    void zeroTickReplacementCannotUseAnOldAcceptedNativeTexture() {
        var registry = new AtlasAnimationPublications();
        try (var bridge = VulkanicGalBridge.create("rust-vulkan")) {
            registry.register(publication(101), () -> {});
            assertFalse(registry.drain(bridge::stageAtlasAnimationAssets, (i,g,t,v,o) -> {
                fail("unstaged resources cannot submit ticks"); return true;
            }));
            bridge.updateWorldMeshAssets(1, List.of(), List.of(texture(101, 0)), List.of());
            registry.textureAccepted(1, texture(101, 0));
            assertTrue(registry.drain(bridge::stageAtlasAnimationAssets, (i,g,t,v,o) -> true));
            registry.register(publication(101), () -> {});
            assertFalse(registry.drain(bridge::stageAtlasAnimationAssets, (i,g,t,v,o) -> true),
                "an old accepted native ID cannot admit an unpublished replacement with no queued ticks");
            bridge.updateWorldMeshAssets(2, List.of(), List.of(texture(101, 0)), List.of());
            registry.textureAccepted(2, texture(101, 0));
            assertThrows(IllegalStateException.class, () -> registry.drain((i,g,t,s) -> {
                throw new IllegalStateException("declaration staging rejected");
            }, (i,g,t,v,o) -> true));
            assertTrue(registry.drain(bridge::stageAtlasAnimationAssets, (i,g,t,v,o) -> true));
        }
    }
}
