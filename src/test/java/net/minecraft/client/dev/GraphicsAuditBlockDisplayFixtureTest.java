package net.minecraft.client.dev;
import net.minecraft.world.phys.Vec3;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;
class GraphicsAuditBlockDisplayFixtureTest {
    @Test void explicitWaterSpriteSelectsObservationOnlyAndRejectsUnknownNames() {
        String enabled = "mattmc.dev.graphicsAuditWaterCycleCapture";
        String selected = "mattmc.dev.graphicsAuditWaterSprite";
        String oldEnabled = System.getProperty(enabled), oldSelected = System.getProperty(selected);
        try {
            System.setProperty(enabled, "true");
            System.setProperty(selected, "flow");
            assertEquals("minecraft:block/water_flow", GraphicsAuditBlockDisplayFixture.observedSprite().toString());
            System.setProperty(selected, "still");
            assertEquals("minecraft:block/water_still", GraphicsAuditBlockDisplayFixture.observedSprite().toString());
            System.setProperty(selected, "overlay");
            assertThrows(IllegalArgumentException.class, GraphicsAuditBlockDisplayFixture::observedSprite);
            System.setProperty(enabled, "false");
            assertEquals("minecraft:block/magma", GraphicsAuditBlockDisplayFixture.observedSprite().toString());
        } finally {
            if (oldEnabled == null) System.clearProperty(enabled); else System.setProperty(enabled, oldEnabled);
            if (oldSelected == null) System.clearProperty(selected); else System.setProperty(selected, oldSelected);
        }
    }
    @Test void waterObservationSelectsOnlyTheDiagnosticSpriteAndPhase() {
        String key = "mattmc.dev.graphicsAuditWaterCycleCapture";
        String phaseKey = "mattmc.dev.graphicsAuditWaterCapturePhase";
        String old = System.getProperty(key);
        String oldPhase = System.getProperty(phaseKey);
        try {
            System.setProperty(key, "true");
            System.setProperty(phaseKey, "2");
            assertEquals("minecraft:block/water_still", GraphicsAuditBlockDisplayFixture.observedSprite().toString());
            assertEquals(2, GraphicsAuditPhaseWait.requestedPhase(64));
            System.setProperty(phaseKey, "64");
            assertThrows(IllegalStateException.class, () -> GraphicsAuditPhaseWait.requestedPhase(64));
            System.setProperty(key, "false");
            assertEquals("minecraft:block/magma", GraphicsAuditBlockDisplayFixture.observedSprite().toString());
        } finally {
            if (old == null) System.clearProperty(key); else System.setProperty(key, old);
            if (oldPhase == null) System.clearProperty(phaseKey); else System.setProperty(phaseKey, oldPhase);
        }
    }
    @Test void placementUsesNormalizedLookAndVanillaUnitBlockOrigin() {
        assertEquals(new Vec3(9.5, 19.5, 33.5),
            GraphicsAuditBlockDisplayFixture.position(new Vec3(10, 20, 30), new Vec3(0, 0, 5)));
        assertEquals(new Vec3(9.5, 19.5, 33.5),
            GraphicsAuditBlockDisplayFixture.position(new Vec3(10, 20, 30), Vec3.ZERO));
        assertEquals(new Vec3(5.5, 19.5, 29.5),
            GraphicsAuditBlockDisplayFixture.position(new Vec3(10, 20, 30), new Vec3(-1, 0, 0)));
    }
}
