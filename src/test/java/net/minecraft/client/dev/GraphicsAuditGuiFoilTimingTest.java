package net.minecraft.client.dev;

import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class GraphicsAuditGuiFoilTimingTest {
    @Test void phaseWindowUsesObservedTicksAndBothAxisJointPeriod() {
        assertTrue(GraphicsAuditGuiFoilTiming.phaseMatches(10000,10000));
        assertTrue(GraphicsAuditGuiFoilTiming.phaseMatches(10512,10000));
        assertFalse(GraphicsAuditGuiFoilTiming.phaseMatches(10513,10000));
        assertFalse(GraphicsAuditGuiFoilTiming.phaseMatches(9999,10000));
        assertTrue(GraphicsAuditGuiFoilTiming.phaseMatches(340000,10000));
        assertFalse(GraphicsAuditGuiFoilTiming.phaseMatches(40000,10000));
        assertFalse(GraphicsAuditGuiFoilTiming.phaseMatches(120000,10000));
        assertFalse(GraphicsAuditGuiFoilTiming.phaseMatches(-1,10000));
    }
    private static void withTrace(Runnable test) {
        String old = System.getProperty("mattmc.dev.guiItemRasterTrace");
        System.setProperty("mattmc.dev.guiItemRasterTrace", "true");
        GraphicsAuditGuiFoilTiming.resetForTest();
        try { test.run(); } finally {
            GraphicsAuditGuiFoilTiming.resetForTest();
            if (old == null) System.clearProperty("mattmc.dev.guiItemRasterTrace");
            else System.setProperty("mattmc.dev.guiItemRasterTrace", old);
        }
    }

    @Test void immutableSnapshotRetainsOnlyTheObservedFrameAndExactInputs() {
        withTrace(() -> {
            GraphicsAuditGuiFoilTiming.beginFrame();
            GraphicsAuditGuiFoilTiming.observeScaledTicks(999,8);
            assertTrue(GraphicsAuditGuiFoilTiming.snapshot().contains("\"samples\":[]"));
            GraphicsAuditGuiFoilTiming.beginItem(12,34);
            GraphicsAuditGuiFoilTiming.observeScaledTicks(49380,8);
            assertTrue(GraphicsAuditGuiFoilTiming.snapshot().contains("\"complete\":false"));
            GraphicsAuditGuiFoilTiming.endItem();
            String captured = GraphicsAuditGuiFoilTiming.snapshot();
            assertTrue(captured.contains("\"complete\":true"));
            assertTrue(captured.contains("\"x\":12,\"y\":34,\"scaledTicks\":49380"));
            GraphicsAuditGuiFoilTiming.beginFrame();
            GraphicsAuditGuiFoilTiming.observeSemanticClock(56,78,12345,.5,.25F);
            String later = GraphicsAuditGuiFoilTiming.snapshot();
            assertFalse(later.contains("scaledTicks"));
            assertTrue(later.contains("\"clockMillis\":12345,\"speed\":0.5,\"strength\":0.25"));
            assertTrue(captured.contains("\"frameSequence\":1"));
            assertTrue(later.contains("\"frameSequence\":2"));
        });
    }

    @Test void nestedUnfinishedAndUnsupportedScopesAreInvalidEvidence() {
        withTrace(() -> {
            GraphicsAuditGuiFoilTiming.beginItem(1,2);
            GraphicsAuditGuiFoilTiming.beginItem(3,4);
            GraphicsAuditGuiFoilTiming.endItem();
            assertTrue(GraphicsAuditGuiFoilTiming.snapshot().contains("\"complete\":false"));
            GraphicsAuditGuiFoilTiming.beginFrame();
            GraphicsAuditGuiFoilTiming.beginItem(1,2);
            GraphicsAuditGuiFoilTiming.observeScaledTicks(1,.5F);
            GraphicsAuditGuiFoilTiming.endItem();
            assertTrue(GraphicsAuditGuiFoilTiming.snapshot().contains("\"complete\":false"));
            GraphicsAuditGuiFoilTiming.beginFrame();
            GraphicsAuditGuiFoilTiming.observeSemanticClock(1,2,-1,.5,.5F);
            assertTrue(GraphicsAuditGuiFoilTiming.snapshot().contains("\"complete\":false"));
            GraphicsAuditGuiFoilTiming.beginFrame();
            GraphicsAuditGuiFoilTiming.beginItem(1,2);
            GraphicsAuditGuiFoilTiming.beginFrame();
            assertTrue(GraphicsAuditGuiFoilTiming.snapshot().contains("\"complete\":false"));
        });
    }

    @Test void observationsAreBoundedAndDisabledMeansNoCollection() {
        withTrace(() -> {
            GraphicsAuditGuiFoilTiming.beginFrame();
            for (int i=0;i<65;i++) GraphicsAuditGuiFoilTiming.observeSemanticClock(i,0,i,.5,.5F);
            String json = GraphicsAuditGuiFoilTiming.snapshot();
            assertTrue(json.contains("\"complete\":false"));
            assertTrue(json.contains("\"x\":63"));
            assertFalse(json.contains("\"x\":64"));
            GraphicsAuditGuiFoilTiming.resetForTest();
            System.setProperty("mattmc.dev.guiItemRasterTrace","false");
            GraphicsAuditGuiFoilTiming.beginFrame();
            GraphicsAuditGuiFoilTiming.beginItem(1,2);
            GraphicsAuditGuiFoilTiming.observeScaledTicks(1,8);
            GraphicsAuditGuiFoilTiming.observeSemanticClock(1,2,1,.5,.5F);
            assertTrue(GraphicsAuditGuiFoilTiming.snapshot().contains("\"samples\":[]"));
            assertTrue(GraphicsAuditGuiFoilTiming.snapshot().contains("\"frameSequence\":0"));
        });
    }
}
