package net.minecraft.client.dev;

import net.minecraft.world.phys.Vec3;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class GraphicsAuditMixedItemFoilFixtureTest {
    @Test void positionUsesCameraDataWithoutRendererState() {
        var p = GraphicsAuditMixedItemFoilFixture.position(new Vec3(0,2,0),new Vec3(0,0,1));
        assertEquals(1.3,p.x,1e-9);
        assertEquals(1.6,p.y,1e-9);
        assertEquals(4,p.z,1e-9);
        assertEquals(p,GraphicsAuditMixedItemFoilFixture.position(new Vec3(0,2,0),Vec3.ZERO));
        var vertical=GraphicsAuditMixedItemFoilFixture.position(Vec3.ZERO,new Vec3(0,1,0));
        assertTrue(Double.isFinite(vertical.x) && Double.isFinite(vertical.y) && Double.isFinite(vertical.z));
    }
    @Test void disabledFixtureDoesNotAccessClientOrRenderer() {
        String key="mattmc.dev.graphicsAuditMixedItemFoil", previous=System.getProperty(key);
        try {
            System.clearProperty(key);
            assertFalse(GraphicsAuditMixedItemFoilFixture.requested());
            assertDoesNotThrow(() -> GraphicsAuditMixedItemFoilFixture.install(null));
            assertEquals("null",GraphicsAuditMixedItemFoilFixture.receipt(null));
        } finally {
            if(previous==null) System.clearProperty(key); else System.setProperty(key,previous);
        }
    }
}
