package net.vulkanic.world;

import java.util.*;
import net.vulkanic.bridge.VulkanicGalBridge;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class ParticleTexturePublicationFailureTest {
    @org.junit.jupiter.api.BeforeAll static void bootstrap() {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
    }
    private static Object field(String name) throws Exception {
        var f = RustGalWorldPrimitiveRenderer.class.getDeclaredField(name);
        f.setAccessible(true); return f.get(null);
    }
    private static VulkanicGalBridge.WorldMeshTextureAssetRecord texture(int id, int color) throws Exception {
        var image = new java.awt.image.BufferedImage(1, 1, java.awt.image.BufferedImage.TYPE_INT_ARGB);
        image.setRGB(0, 0, color);
        var output = new java.io.ByteArrayOutputStream();
        assertTrue(javax.imageio.ImageIO.write(image, "PNG", output));
        return new VulkanicGalBridge.WorldMeshTextureAssetRecord(id, output.toByteArray(), List.of());
    }
    @SuppressWarnings({"unchecked", "rawtypes"})
    @Test void nativeRejectionCannotUseOldParticleTextureAndSuccessfulRetryRestoresAdmission() throws Exception {
        var textures = (Map<Integer, VulkanicGalBridge.WorldMeshTextureAssetRecord>) field("WORLD_MESH_TEXTURES");
        var dirty = (Set<Integer>) field("DIRTY_WORLD_MESH_TEXTURES");
        var uploaded = (Map<Integer, Long>) field("UPLOADED_WORLD_MESH_TEXTURES");
        var savedMaps = new IdentityHashMap<Map, Map>();
        var savedSets = new IdentityHashMap<Set, Set>();
        for (String name : List.of("WORLD_MESH_TEXTURES", "DIRTY_WORLD_MESH_TEXTURES",
            "UPLOADED_WORLD_MESH_TEXTURES", "DIRTY_WORLD_MESH_ASSETS", "DIRTY_WORLD_MESH_SORTED_INDICES",
            "PENDING_WORLD_MESH_RETIREMENTS")) {
            Object value = field(name);
            if (value instanceof Map map) { savedMaps.put(map, new LinkedHashMap(map)); map.clear(); }
            else if (value instanceof Set set) { savedSets.put(set, new LinkedHashSet(set)); set.clear(); }
            else fail("unexpected registry type: " + name);
        }
        int id = RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_PARTICLE_ATLAS;
        var quad = new VulkanicGalBridge.WorldMaterialQuadRecord(1, 1, id, 0, 0, 0, 0, 0, -1,
            0,0,0, 1,0,0, 1,1,0, 0,1,0, 0,0, 1,0, 1,1, 0,1,
            1280,720, RustGalWorldPrimitiveRenderer.MATERIAL_SOURCE_PARTICLES, 0, -1, 240);
        var quads = List.of(quad);
        try (var bridge = VulkanicGalBridge.create("rust-vulkan")) {
            textures.put(id, texture(id, 0xffff0000)); dirty.add(id);
            assertThrows(IllegalStateException.class, () -> RustGalWorldPrimitiveRenderer.requireAcceptedParticleTextures(quads));
            assertNotNull(RustGalWorldPrimitiveRenderer.flushPendingWorldMeshAssets(bridge));
            assertTrue(uploaded.containsKey(id));
            long firstGeneration = RustGalWorldPrimitiveRenderer.requireAcceptedWorldMeshTextureGeneration(id);
            var firstPayload = textures.get(id);
            assertEquals(firstGeneration, RustGalWorldPrimitiveRenderer.requireAcceptedWorldMeshTextureGeneration(firstPayload));
            assertTrue(firstGeneration > 0);
            assertDoesNotThrow(() -> bridge.updateGuiAtlasReferences(1, List.of(
                new VulkanicGalBridge.GuiAtlasReferenceRecord(101, id, firstGeneration, 1, 1, 0, 0, 1, 1))));
            int unrelatedId = 0x74657374;
            textures.put(unrelatedId, texture(unrelatedId, 0xff0000ff));
            dirty.add(unrelatedId);
            assertNotNull(RustGalWorldPrimitiveRenderer.flushPendingWorldMeshAssets(bridge));
            assertTrue(RustGalWorldPrimitiveRenderer.requireAcceptedWorldMeshTextureGeneration(unrelatedId) > firstGeneration);
            assertEquals(firstGeneration, RustGalWorldPrimitiveRenderer.requireAcceptedWorldMeshTextureGeneration(id),
                "an unrelated upload must not relabel this atlas incarnation");
            assertDoesNotThrow(() -> bridge.updateGuiAtlasReferences(1, List.of(
                new VulkanicGalBridge.GuiAtlasReferenceRecord(101, id, firstGeneration, 1, 1, 0, 0, 1, 1))));
            assertDoesNotThrow(() -> RustGalWorldPrimitiveRenderer.requireAcceptedParticleTextures(quads));

            // Deliberately corrupt the copied wire payload, bypassing Java's PNG
            // input validation so the actual native decoder rejects replacement.
            textures.put(id, new VulkanicGalBridge.WorldMeshTextureAssetRecord(id, new byte[]{1,2,3}, List.of()));
            dirty.add(id);
            assertNull(RustGalWorldPrimitiveRenderer.flushPendingWorldMeshAssets(bridge));
            assertEquals(firstGeneration, uploaded.get(id), "failed upload retains the previous accepted generation");
            assertThrows(IllegalStateException.class,
                () -> RustGalWorldPrimitiveRenderer.requireAcceptedWorldMeshTextureGeneration(id));
            assertTrue(dirty.contains(id), "failed replacement remains pending");
            assertThrows(IllegalStateException.class, () -> RustGalWorldPrimitiveRenderer.requireAcceptedParticleTextures(quads));

            textures.put(id, texture(id, 0xff00ff00));
            assertNotNull(RustGalWorldPrimitiveRenderer.flushPendingWorldMeshAssets(bridge));
            assertFalse(dirty.contains(id));
            long replacementGeneration = RustGalWorldPrimitiveRenderer.requireAcceptedWorldMeshTextureGeneration(id);
            assertThrows(IllegalStateException.class,
                () -> RustGalWorldPrimitiveRenderer.requireAcceptedWorldMeshTextureGeneration(firstPayload));
            assertEquals(replacementGeneration,
                RustGalWorldPrimitiveRenderer.requireAcceptedWorldMeshTextureGeneration(textures.get(id)));
            assertTrue(replacementGeneration > firstGeneration);
            assertThrows(RuntimeException.class, () -> bridge.updateGuiAtlasReferences(2, List.of(
                new VulkanicGalBridge.GuiAtlasReferenceRecord(101, id, firstGeneration, 1, 1, 0, 0, 1, 1))));
            assertDoesNotThrow(() -> bridge.updateGuiAtlasReferences(2, List.of(
                new VulkanicGalBridge.GuiAtlasReferenceRecord(101, id, replacementGeneration, 1, 1, 0, 0, 1, 1))));
            assertDoesNotThrow(() -> RustGalWorldPrimitiveRenderer.requireAcceptedParticleTextures(quads));
            textures.remove(id);
            assertThrows(IllegalStateException.class,
                () -> RustGalWorldPrimitiveRenderer.requireAcceptedWorldMeshTextureGeneration(id));
            assertThrows(IllegalStateException.class, () -> RustGalWorldPrimitiveRenderer.requireAcceptedParticleTextures(quads));
        } finally {
            savedMaps.forEach((map, saved) -> { map.clear(); map.putAll(saved); });
            savedSets.forEach((set, saved) -> { set.clear(); set.addAll(saved); });
        }
    }
}
