package net.vulkanic.world;

import java.util.List;
import java.util.Map;
import net.minecraft.client.Minecraft;
import net.minecraft.client.particle.SingleQuadParticle;
import net.minecraft.client.renderer.state.QuadParticleRenderState;
import net.vulkanic.VulkanicAPI;
import net.vulkanic.backends.vulkan.VulkanWholeFrameSemanticGpuDevice;
import net.vulkanic.bridge.VulkanicGalBridge;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class ParticleAtlasAdmissionTest {
    @org.junit.jupiter.api.BeforeAll static void bootstrap() {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
    }
    private static java.lang.reflect.Field field(Class<?> type, String name) throws Exception {
        var field = type.getDeclaredField(name); field.setAccessible(true); return field;
    }

    @Test void cachedDefaultAtlasDoesNotAdmitQuadsWhenItsCurrentSourceCannotBePublished() throws Exception {
        var device = field(VulkanicAPI.class, "device");
        var client = field(Minecraft.class, "instance");
        var width = field(RustGalWorldPrimitiveRenderer.class, "pendingViewportWidth");
        var height = field(RustGalWorldPrimitiveRenderer.class, "pendingViewportHeight");
        @SuppressWarnings("unchecked")
        var textures = (Map<Integer, VulkanicGalBridge.WorldMeshTextureAssetRecord>) field(
            RustGalWorldPrimitiveRenderer.class, "WORLD_MESH_TEXTURES").get(null);
        int id = RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_PARTICLE_ATLAS;
        var oldTexture = textures.get(id);
        var oldDevice = device.get(null); var oldClient = client.get(null);
        int oldWidth = width.getInt(null), oldHeight = height.getInt(null);
        String property = "mattmc.dev.rustGalVulkanWholeFrame";
        String oldProperty = System.getProperty(property);
        int checkpoint = RustGalWorldPrimitiveRenderer.markMaterialQuadBatch();
        try {
            System.setProperty(property, "true");
            device.set(null, new VulkanWholeFrameSemanticGpuDevice());
            client.set(null, null); // no current resource can validate the stale cached ID
            width.setInt(null, 1280); height.setInt(null, 720);
            var png = new java.io.ByteArrayOutputStream();
            assertTrue(javax.imageio.ImageIO.write(new java.awt.image.BufferedImage(1, 1,
                java.awt.image.BufferedImage.TYPE_INT_ARGB), "PNG", png));
            var stale = new VulkanicGalBridge.WorldMeshTextureAssetRecord(id, png.toByteArray(), List.of());
            textures.put(id, stale);
            assertTrue(WorldRenderRoutePolicy.currentMaterialRoute().usesRustWholeFrameVulkan());
            var state = new QuadParticleRenderState();
            state.add(SingleQuadParticle.Layer.OPAQUE, 0, 0, 0, 0, 0, 0, 1, .25F, 0, 1, 0, 1, -1, 240);
            state.add(SingleQuadParticle.Layer.TRANSLUCENT, 0, 0, 0, 0, 0, 0, 1, .25F, 0, 1, 0, 1, -1, 240);
            assertThrows(IllegalStateException.class, state::enqueueRustGal);
            assertEquals(checkpoint, RustGalWorldPrimitiveRenderer.markMaterialQuadBatch(),
                "default atlas preflight must reject before either layer queues geometry");
            assertThrows(IllegalStateException.class, () -> RustGalWorldPrimitiveRenderer.enqueueParticleQuad(
                false, 0, 0, 0, 0, 0, 0, 1, .25F, 0, 1, 0, 1, -1, 240));
            assertEquals(checkpoint, RustGalWorldPrimitiveRenderer.markMaterialQuadBatch(),
                "direct callers cannot bypass current-incarnation publication with a cached ID");
            assertSame(stale, textures.get(id), "rejection does not destroy the retained asset");
        } finally {
            RustGalWorldPrimitiveRenderer.rollbackMaterialQuadBatch(checkpoint);
            if (oldTexture == null) textures.remove(id); else textures.put(id, oldTexture);
            device.set(null, oldDevice); client.set(null, oldClient);
            width.setInt(null, oldWidth); height.setInt(null, oldHeight);
            if (oldProperty == null) System.clearProperty(property); else System.setProperty(property, oldProperty);
        }
    }
}
