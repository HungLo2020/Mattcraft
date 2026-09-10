package net.minecraft.client.dev;

import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class GraphicsAuditCowOutlineFixtureTest {
    @Test
    void optInRequiresAnObservedEntityAndDisabledFixtureDoesNotTouchRendererOrWorld() {
        String property = "mattmc.dev.graphicsAuditCowGlowing";
        String previous = System.getProperty(property);
        try {
            System.clearProperty(property);
            assertFalse(GraphicsAuditCowOutlineFixture.requested());
            GraphicsAuditCowOutlineFixture.configure(null);
            assertTrue(GraphicsAuditCowOutlineFixture.ready(null));
            assertFalse(GraphicsAuditCowOutlineFixture.receipt(null).get("entityPresent").getAsBoolean());
            System.setProperty(property, "true");
            assertTrue(GraphicsAuditCowOutlineFixture.requested());
            assertFalse(GraphicsAuditCowOutlineFixture.ready(null));
            assertFalse(GraphicsAuditCowOutlineFixture.receipt(null).get("ready").getAsBoolean());
        } finally {
            if (previous == null) System.clearProperty(property); else System.setProperty(property, previous);
        }
    }
}
