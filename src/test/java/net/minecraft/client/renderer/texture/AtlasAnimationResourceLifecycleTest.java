package net.minecraft.client.renderer.texture;

import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.concurrent.CompletableFuture;
import net.blaze3d.platform.NativeImage;
import net.minecraft.client.resources.metadata.animation.AnimationMetadataSection;
import net.minecraft.client.resources.metadata.animation.FrameSize;
import net.minecraft.resources.ResourceLocation;
import net.sodium.client.render.texture.SpriteUtilImpl;
import net.vulkanic.VulkanicAPI;
import net.vulkanic.backends.vulkan.VulkanWholeFrameSemanticGpuDevice;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class AtlasAnimationResourceLifecycleTest {
    @org.junit.jupiter.api.BeforeAll
    static void bootstrap() {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
    }

    private static SpriteLoader.Preparations preparations(net.minecraft.resources.ResourceLocation location) {
        var name = ResourceLocation.withDefaultNamespace("audit/animated");
        var contents = new SpriteContents(name, new FrameSize(1, 1), new NativeImage(1, 2, true),
            Optional.of(new AnimationMetadataSection(Optional.empty(), Optional.of(1), Optional.of(1), 2, true)),
            List.of());
        var animated = new TextureAtlasSprite(location, contents, 32, 16, 16, 0);
        var missing = new TextureAtlasSprite(location, MissingTextureAtlasSprite.create(), 32, 16, 0, 0);
        return new SpriteLoader.Preparations(32, 16, 0, missing,
            Map.of(name, animated, MissingTextureAtlasSprite.getLocation(), missing), CompletableFuture.completedFuture(null));
    }

    @Test
    void realAtlasUploadBindsUsesBeforeWorldPublicationAndRetiresOldSpriteIdentity() throws Exception {
        verifyAtlasLifecycle(TextureAtlas.LOCATION_BLOCKS);
        verifyAtlasLifecycle(TextureAtlas.LOCATION_PARTICLES);
    }

    private void verifyAtlasLifecycle(net.minecraft.resources.ResourceLocation location) throws Exception {
        // Install only the CPU semantic device; no Java or borrowed GPU context.
        // Restore static test scaffolding even if upload or an assertion fails.
        var device = VulkanicAPI.class.getDeclaredField("device");
        device.setAccessible(true);
        var previousDevice = device.get(null);
        String property = "mattmc.dev.rustGalVulkanWholeFrame";
        String previousMode = System.getProperty(property);
        String tickProperty = "mattmc.dev.rustGalAtlasAnimation";
        String previousTicks = System.getProperty(tickProperty);
        var config = net.sodium.client.SodiumClientMod.class.getDeclaredField("CONFIG");
        config.setAccessible(true);
        var previousConfig = config.get(null);
        TextureAtlas atlas = null;
        try {
            System.setProperty(property, "true");
            System.clearProperty(tickProperty);
            if (location.equals(TextureAtlas.LOCATION_PARTICLES)) System.setProperty(tickProperty, "true");
            config.set(null, net.sodium.client.gui.SodiumGameOptions.defaults());
            device.set(null, new VulkanWholeFrameSemanticGpuDevice());
            atlas = new TextureAtlas(location);
            assertEquals(0, atlas.width);
            assertEquals(0, atlas.height);
            assertNull(atlas.semanticAnimationResource(),
                "the startup pump must defer until upload publishes an immutable incarnation");
            var first = preparations(location);
            atlas.upload(first);
            var oldResource = atlas.semanticAnimationResource();
            assertNotNull(oldResource);
            assertEquals(location, oldResource.atlas());
            assertEquals(location.equals(TextureAtlas.LOCATION_BLOCKS)
                ? net.vulkanic.world.RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_TERRAIN_BLOCK_ATLAS
                : net.vulkanic.world.RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_PARTICLE_ATLAS,
                oldResource.semanticTextureId());
            assertSame(oldResource.source(), atlas.semanticAnimationSource());
            assertNull(atlas.texture);
            assertNull(atlas.textureView);
            var name = oldResource.source().sprites().getFirst().name();
            var oldSprite = first.regions().get(name);
            assertSame(oldResource, oldSprite.semanticAnimationResource());
            assertNull(first.missing().semanticAnimationResource(), "static sprites need no animation event identity");
            new SpriteUtilImpl().markSpriteActive(oldSprite);
            assertFalse(oldResource.recordUse(location, name),
                "the real use hook must collect before a world publication exists");
            oldResource.enqueueTick(1, true);
            var tickers = TextureAtlas.class.getDeclaredField("animatedTextures");
            tickers.setAccessible(true);
            tickers.set(atlas, List.of(new TextureAtlasSprite.Ticker() {
                @Override public void tickAndUpload(net.blaze3d.textures.GpuTexture texture) {
                    fail("selected Vulkan must not invoke a stale Java uploader");
                }
                @Override public boolean tickSemantic() {
                    return fail("selected Vulkan must not advance a stale Java animation clock");
                }
                @Override public void close() {}
            }));
            System.clearProperty(tickProperty);
            atlas.cycleAnimationFrames();
            System.setProperty(tickProperty, "true");
            atlas.cycleAnimationFrames();
            assertThrows(IllegalArgumentException.class, () -> oldResource.enqueueTick(2, true),
                "the private real texture tick must enqueue exactly the next semantic event");
            oldResource.enqueueTick(3, false);
            var second = preparations(location);
            atlas.upload(second);
            var replacement = atlas.semanticAnimationResource();
            assertNotSame(oldResource, replacement);
            assertTrue(replacement.source().generation() > oldResource.source().generation());
            assertThrows(IllegalStateException.class, () -> oldResource.enqueueTick(2, true));
            new SpriteUtilImpl().markSpriteActive(oldSprite);
            assertTrue(replacement.recordUse(location, name),
                "a late old-sprite use must not activate the same name in the replacement");
            replacement.enqueueTick(1, true);
            new SpriteUtilImpl().markSpriteActive(second.regions().get(name));
            assertFalse(replacement.recordUse(location, name));
            atlas.clearTextureData();
            assertNull(atlas.semanticAnimationResource());
            assertFalse(replacement.recordUse(location, name));
            assertThrows(IllegalStateException.class, () -> replacement.enqueueTick(2, true));
        } finally {
            if (atlas != null) atlas.clearTextureData();
            device.set(null, previousDevice);
            config.set(null, previousConfig);
            if (previousTicks == null) System.clearProperty(tickProperty);
            else System.setProperty(tickProperty, previousTicks);
            if (previousMode == null) System.clearProperty(property);
            else System.setProperty(property, previousMode);
        }
    }

    @Test
    void resourcePumpChecksIncarnationBeforePublicationAndDoesNotPresent() throws Exception {
        String source = java.nio.file.Files.readString(java.nio.file.Path.of(
            "src/main/java/net/vulkanic/gui/RustGalFrameCoordinator.java"));
        int start = source.indexOf("public static void pumpAtlasAnimationResources()");
        String pump = source.substring(start, source.indexOf("private static void flushPendingWorldAssetsLocked()", start));
        int ready = pump.indexOf("if (atlas.semanticAnimationResource() != null)");
        assertTrue(ready >= 0);
        assertTrue(ready < pump.indexOf("ensureTerrainAtlasAssetForWorldMesh()"));
        assertTrue(ready < pump.indexOf("flushPendingAtlasAnimationTicks(bridge)"));
        assertTrue(pump.indexOf("ensureParticleAtlasAnimationAsset()") > pump.indexOf("ensureTerrainAtlasAssetForWorldMesh()"));
        assertTrue(pump.indexOf("ensureParticleAtlasAnimationAsset()") < pump.indexOf("flushPendingWorldMeshAssets(bridge)"));
        assertFalse(pump.contains("presentFrame("));
        assertFalse(pump.contains("executeFrameBatches("));
    }
}
