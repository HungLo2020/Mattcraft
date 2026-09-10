package net.vulkanic.world;

import java.util.List;
import net.minecraft.client.renderer.texture.SemanticAtlasAnimationSource;
import net.minecraft.client.renderer.texture.SpriteContents;
import net.minecraft.resources.ResourceLocation;
import net.vulkanic.bridge.VulkanicGalBridge;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class AtlasAnimationResourceIdentityTest {
    @org.junit.jupiter.params.ParameterizedTest
    @org.junit.jupiter.params.provider.ValueSource(ints = {17, 0xfdf71712, 0x80000000, 0xffffffff})
    void unsignedSemanticIdentitySurvivesPublicationAndTickDelivery(int textureId) {
        try (var resource = new AtlasAnimationResource(GUI, textureId, source())) {
            var texture = new VulkanicGalBridge.WorldMeshTextureAssetRecord(textureId, new byte[]{1}, List.of());
            var publication = new AtlasAnimationPublication(texture, resource);
            assertTrue(publication.recordSpriteUse(GUI, SPRITE));
            publication.enqueueTick(1, true);
            publication.textureAccepted(33, texture);
            publication.flush((id, generation, initialTick, snapshot) -> {
                assertEquals(textureId, id);
                assertEquals(33, generation);
                assertSame(resource.source(), snapshot);
                return null;
            });
            assertTrue(publication.drainTicks((id, generation, tick, visible, onlyVisible) -> {
                assertEquals(textureId, id);
                assertEquals(33, generation);
                assertEquals(1, tick);
                assertArrayEquals(new int[]{1}, visible);
                return true;
            }));
        }
    }

    @Test void zeroIsNotASemanticTextureIdentity() {
        assertThrows(IllegalArgumentException.class, () -> new AtlasAnimationResource(GUI, 0, source()));
        assertThrows(IllegalArgumentException.class, () -> new AtlasAnimationTickDelivery(0, 1, 0));
    }

    private static final ResourceLocation BLOCKS = ResourceLocation.withDefaultNamespace("textures/atlas/blocks.png");
    private static final ResourceLocation GUI = ResourceLocation.withDefaultNamespace("textures/atlas/gui.png");
    private static final ResourceLocation SPRITE = ResourceLocation.withDefaultNamespace("audit/shared-name");
    private static SemanticAtlasAnimationSource source() {
        return new SemanticAtlasAnimationSource(7, 1, 1, 1, List.of(
            new SemanticAtlasAnimationSource.Sprite(1, SPRITE, 0, 0,
                new SpriteContents.SemanticAnimationSource(1, 1, 2, false,
                    List.of(new SpriteContents.SemanticAnimationFrame(0, 1),
                            new SpriteContents.SemanticAnimationFrame(1, 1)),
                    List.of(new SpriteContents.SemanticAnimationMip(1, 2,
                        new byte[]{10, 20, 30, -1, 40, 50, 60, -1}))))));
    }

    @Test void semanticItemUseReachesTheOwnedTickWithoutSelectingAJavaFrame() {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        try (var resource = new AtlasAnimationResource(BLOCKS, 101, source())) {
            RustGalWorldPrimitiveRenderer.recordAtlasSpriteUse(resource, BLOCKS, SPRITE);
            RustGalWorldPrimitiveRenderer.recordAtlasSpriteUse(resource, BLOCKS, SPRITE);
            resource.enqueueNextTick(true);
            assertTrue(resource.drain(11, (id, generation, tick, visible, onlyVisible) -> {
                assertEquals(101,id);
                assertEquals(1,tick);
                assertArrayEquals(new int[]{1},visible,"repeated GUI uses form one immutable sprite ID");
                assertTrue(onlyVisible);
                return true;
            }));
            resource.enqueueNextTick(true);
            assertTrue(resource.drain(11, (id, generation, tick, visible, onlyVisible) -> {
                assertArrayEquals(new int[0],visible,"uses must not leak into later unseen frames");
                return true;
            }));
        }
    }

    @Test void identicallyNamedSpritesRemainOwnedByTheirExplicitAtlasAndAsset() {
        try (var blocks = new AtlasAnimationResource(BLOCKS, 101, source());
             var gui = new AtlasAnimationResource(GUI, 202, source())) {
            assertFalse(blocks.recordUse(GUI, SPRITE));
            assertFalse(gui.recordUse(BLOCKS, SPRITE));
            assertTrue(blocks.recordUse(BLOCKS, SPRITE));
            blocks.enqueueNextTick(true);
            gui.enqueueNextTick(true);
            assertTrue(blocks.drain(11, (id, generation, tick, visible, onlyVisible) -> {
                assertEquals(101, id); assertEquals(11, generation); assertEquals(1, tick);
                assertArrayEquals(new int[]{1}, visible); assertTrue(onlyVisible);
                return true;
            }));
            assertTrue(gui.drain(22, (id, generation, tick, visible, onlyVisible) -> {
                assertEquals(202, id); assertEquals(22, generation); assertEquals(1, tick);
                assertArrayEquals(new int[0], visible);
                return true;
            }));
            blocks.close();
            assertTrue(gui.recordUse(GUI, SPRITE), "retiring a different atlas must not consume this atlas's uses");
            gui.enqueueNextTick(true);
            assertTrue(gui.drain(22, (id, generation, tick, visible, onlyVisible) -> {
                assertEquals(202, id); assertEquals(2, tick);
                assertArrayEquals(new int[]{1}, visible);
                return true;
            }));
        }
    }

    @Test void publicationAndTickTransportUseTheSameExplicitSemanticIdentity() {
        try (var resource = new AtlasAnimationResource(GUI, 202, source())) {
            assertThrows(IllegalArgumentException.class, () -> new AtlasAnimationPublication(
                new VulkanicGalBridge.WorldMeshTextureAssetRecord(101, new byte[]{1}, List.of()), resource));
            assertThrows(IllegalArgumentException.class, () -> new AtlasAnimationPublication(
                new VulkanicGalBridge.WorldMeshTextureAssetRecord(202, new byte[]{1}), resource),
                "an unspecified mip allocation is not an exact animated-atlas declaration");
            var texture = new VulkanicGalBridge.WorldMeshTextureAssetRecord(202, new byte[]{1}, List.of());
            assertEquals(1, texture.requestedMipLevels());
            assertEquals(3, new VulkanicGalBridge.WorldMeshTextureAssetRecord(202, new byte[]{1},
                List.of(new byte[]{2}, new byte[]{3})).requestedMipLevels());
            var publication = new AtlasAnimationPublication(texture, resource);
            assertTrue(publication.recordSpriteUse(GUI, SPRITE));
            publication.enqueueTick(1, true);
            publication.textureAccepted(33, texture);
            publication.flush((id, generation, initialTick, source) -> {
                assertEquals(202, id); assertEquals(33, generation); assertEquals(0, initialTick);
                assertSame(resource.source(), source);
                return null;
            });
            assertTrue(publication.drainTicks((id, generation, tick, visible, onlyVisible) -> {
                assertEquals(202, id); assertEquals(33, generation); assertEquals(1, tick);
                assertArrayEquals(new int[]{1}, visible);
                return true;
            }));
        }
    }

    @Test void invalidIdentitiesRejectBeforeEventsCanBeCollected() {
        assertThrows(IllegalArgumentException.class, () -> new AtlasAnimationResource(GUI, 0, source()));
        assertThrows(NullPointerException.class, () -> new AtlasAnimationResource(null, 202, source()));
    }
}
