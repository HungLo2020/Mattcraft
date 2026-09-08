package net.vulkanic.world;

import net.minecraft.client.renderer.chunk.ChunkSectionLayer;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class TerrainCameraSortPolicyTest {
    @Test void translucentCameraOrderingDoesNotDependOnDepthWrites() {
        assertTrue(ChunkSectionLayer.TRANSLUCENT.pipeline().isWriteDepth());
        assertEquals(RustGalWorldPrimitiveRenderer.DEPTH_POLICY_TEST_WRITE,
            RustGalTerrainRenderer.terrainDepthPolicy(ChunkSectionLayer.TRANSLUCENT));
        assertTrue(RustGalTerrainRenderer.terrainCameraSortRequested(ChunkSectionLayer.TRANSLUCENT));
        for (var layer : new ChunkSectionLayer[]{ChunkSectionLayer.SOLID, ChunkSectionLayer.CUTOUT,
                ChunkSectionLayer.CUTOUT_MIPPED}) {
            assertFalse(RustGalTerrainRenderer.terrainCameraSortRequested(layer));
        }
    }
}
