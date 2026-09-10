package net.vulkanic.world;

import java.util.List;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.Sheets;
import net.minecraft.client.renderer.texture.SemanticAtlasAnimationSource;
import net.minecraft.client.renderer.texture.SpriteContents;
import net.minecraft.client.renderer.texture.TextureAtlas;
import net.minecraft.client.renderer.texture.TextureAtlasSprite;
import net.minecraft.resources.ResourceLocation;
import net.vulkanic.bridge.VulkanicGalBridge;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;
import static org.mockito.Mockito.*;

class ShieldAtlasSpriteExtractionTest {
    @Test void animatedAdmissionRequiresCurrentBoundResourceAndBothPrivateFlags() {
        String atlasKey = "mattmc.dev.rustGalShieldAtlas", clockKey = "mattmc.dev.rustGalShieldAtlasAnimation";
        String beforeAtlas = System.getProperty(atlasKey), beforeClock = System.getProperty(clockKey);
        var name = ResourceLocation.withDefaultNamespace("entity/shield_base_nopattern");
        var animation = new SpriteContents.SemanticAnimationSource(1, 1, 2, false,
            List.of(new SpriteContents.SemanticAnimationFrame(1, 3), new SpriteContents.SemanticAnimationFrame(0, 5)),
            List.of(new SpriteContents.SemanticAnimationMip(1, 2, new byte[8])));
        var declaration = new SemanticAtlasAnimationSource(7, 1, 1, 1,
            List.of(new SemanticAtlasAnimationSource.Sprite(1, name, 0, 0, animation)));
        try (var game = mockStatic(Minecraft.class);
             var resource = new AtlasAnimationResource(Sheets.SHIELD_SHEET,
                 RustGalWorldPrimitiveRenderer.shieldAtlasTextureId(), declaration)) {
            System.setProperty(atlasKey, "true"); System.setProperty(clockKey, "true");
            var minecraft = mock(Minecraft.class, RETURNS_DEEP_STUBS);
            game.when(Minecraft::getInstance).thenReturn(minecraft);
            var atlas = mock(TextureAtlas.class);
            var sprite = mock(TextureAtlasSprite.class, RETURNS_DEEP_STUBS);
            when(sprite.atlasLocation()).thenReturn(Sheets.SHIELD_SHEET);
            when(sprite.contents().name()).thenReturn(name);
            when(sprite.contents().isAnimated()).thenReturn(true);
            when(minecraft.getTextureManager().getTexture(Sheets.SHIELD_SHEET)).thenReturn(atlas);
            when(atlas.getSprite(name)).thenReturn(sprite);
            when(atlas.semanticAnimationResource()).thenReturn(resource);
            assertFalse(RustGalWorldPrimitiveRenderer.privateOwnedShieldSprite(sprite));
            when(sprite.semanticAnimationResource()).thenReturn(resource);
            assertTrue(RustGalWorldPrimitiveRenderer.privateOwnedShieldSprite(sprite));
            System.clearProperty(clockKey);
            assertFalse(RustGalWorldPrimitiveRenderer.privateOwnedShieldSprite(sprite));
            System.setProperty(clockKey, "true"); System.clearProperty(atlasKey);
            assertFalse(RustGalWorldPrimitiveRenderer.privateOwnedShieldSprite(sprite));
            System.setProperty(atlasKey, "true");
            when(atlas.getSprite(name)).thenReturn(mock(TextureAtlasSprite.class));
            assertFalse(RustGalWorldPrimitiveRenderer.privateOwnedShieldSprite(sprite));
            when(atlas.getSprite(name)).thenReturn(sprite);
            resource.close();
            assertFalse(RustGalWorldPrimitiveRenderer.privateOwnedShieldSprite(sprite));
            verify(atlas, never()).semanticRawSnapshot();
        } finally {
            if (beforeAtlas == null) System.clearProperty(atlasKey); else System.setProperty(atlasKey, beforeAtlas);
            if (beforeClock == null) System.clearProperty(clockKey); else System.setProperty(clockKey, beforeClock);
        }
    }

    @org.junit.jupiter.api.BeforeAll static void bootstrap() {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
    }

    @Test void privateStaticPathRejectsStaleSpritesAndAnyAtlasAnimationBeforePixelCopy() {
        String key = "mattmc.dev.rustGalShieldAtlas", before = System.getProperty(key);
        try (var game = mockStatic(Minecraft.class)) {
            System.clearProperty(key);
            assertThrows(IllegalStateException.class,
                () -> RustGalWorldPrimitiveRenderer.requireShieldAtlasSpritePayload(null));
            System.setProperty(key, "true");
            var minecraft = mock(Minecraft.class, RETURNS_DEEP_STUBS);
            game.when(Minecraft::getInstance).thenReturn(minecraft);
            var atlas = mock(TextureAtlas.class);
            when(minecraft.getTextureManager().getTexture(Sheets.SHIELD_SHEET)).thenReturn(atlas);
            var sprite = mock(TextureAtlasSprite.class);
            var contents = mock(SpriteContents.class);
            var name = ResourceLocation.withDefaultNamespace("entity/shield_base");
            when(sprite.atlasLocation()).thenReturn(Sheets.SHIELD_SHEET);
            when(sprite.contents()).thenReturn(contents);
            when(contents.name()).thenReturn(name);
            when(atlas.getSprite(name)).thenReturn(mock(TextureAtlasSprite.class));
            assertThrows(IllegalStateException.class,
                () -> RustGalWorldPrimitiveRenderer.requireShieldAtlasSpritePayload(sprite));
            when(atlas.getSprite(name)).thenReturn(sprite);
            when(contents.isAnimated()).thenReturn(true);
            assertThrows(IllegalStateException.class,
                () -> RustGalWorldPrimitiveRenderer.requireShieldAtlasSpritePayload(sprite));
            when(contents.isAnimated()).thenReturn(false);
            // Even an animation in another sprite forbids current-frame copying.
            when(atlas.semanticAnimationSource()).thenReturn(new SemanticAtlasAnimationSource(
                1,64,64,1,List.of(new SemanticAtlasAnimationSource.Sprite(1,name,0,0,null))));
            assertThrows(IllegalStateException.class,
                () -> RustGalWorldPrimitiveRenderer.requireShieldAtlasSpritePayload(sprite));
            verify(atlas, never()).semanticRawSnapshot();
            when(sprite.atlasLocation()).thenReturn(TextureAtlas.LOCATION_BLOCKS);
            assertThrows(IllegalStateException.class,
                () -> RustGalWorldPrimitiveRenderer.requireShieldAtlasSpritePayload(sprite));
        } finally {
            if (before == null) System.clearProperty(key); else System.setProperty(key,before);
        }
    }

    @Test void sharedRegionPreservesSemanticPayloadAndRejectsOverflow() {
        var payload = new VulkanicGalBridge.WorldMeshTextureAssetRecord(17,new byte[]{1,2});
        var region = new AtlasSpritePayload(payload,64,32,8,4,16,16);
        assertSame(payload,region.texture());
        assertEquals(8,region.x());
        assertEquals(16,region.height());
        assertThrows(IllegalArgumentException.class, () -> new AtlasSpritePayload(payload,64,32,Integer.MAX_VALUE,0,2,2));
        assertThrows(IllegalArgumentException.class, () -> new AtlasSpritePayload(payload,64,32,0,0,65,2));
        assertThrows(IllegalArgumentException.class, () -> new AtlasSpritePayload(payload,64,32,0,0,0,2));
    }

    @Test void publicationReuseRequiresExactResourceAndRegisteredPayloadIncarnation() {
        var atlas = mock(TextureAtlas.class);
        var payload = new VulkanicGalBridge.WorldMeshTextureAssetRecord(17,new byte[]{1,2});
        var published = new RustGalWorldPrimitiveRenderer.ModelAtlasPublication(atlas,3,payload);
        assertTrue(published.matches(atlas,3,payload));
        assertFalse(published.matches(atlas,4,payload));
        assertFalse(published.matches(mock(TextureAtlas.class),3,payload));
        assertFalse(published.matches(atlas,3,null));
        assertFalse(published.matches(atlas,3,new VulkanicGalBridge.WorldMeshTextureAssetRecord(17,new byte[]{1,2})));
    }
}
