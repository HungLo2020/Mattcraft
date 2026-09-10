package net.minecraft.client.dev;

import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class GraphicsAuditHandFoilTimingTest {
    private static void withTrace(Runnable action) {
        String old = System.getProperty("mattmc.dev.handItemFoilTiming");
        System.setProperty("mattmc.dev.handItemFoilTiming", "true");
        GraphicsAuditHandFoilTiming.resetForTest();
        try { action.run(); } finally {
            GraphicsAuditHandFoilTiming.resetForTest();
            if (old == null) System.clearProperty("mattmc.dev.handItemFoilTiming");
            else System.setProperty("mattmc.dev.handItemFoilTiming", old);
        }
    }

    @Test void ignoresWorldAndGuiTicksOutsideHandScope() {
        withTrace(() -> {
            GraphicsAuditHandFoilTiming.beginFrame();
            GraphicsAuditHandFoilTiming.observeScaledTicks(10000,8);
            assertFalse(GraphicsAuditHandFoilTiming.readyForCapture(10000));
            GraphicsAuditHandFoilTiming.beginHand();
            GraphicsAuditHandFoilTiming.observeScaledTicks(10000,8);
            assertFalse(GraphicsAuditHandFoilTiming.readyForCapture(10000));
            GraphicsAuditHandFoilTiming.endHand();
            assertTrue(GraphicsAuditHandFoilTiming.readyForCapture(10000));
            String receipt = GraphicsAuditHandFoilTiming.snapshot();
            GraphicsAuditHandFoilTiming.beginFrame();
            assertFalse(GraphicsAuditHandFoilTiming.readyForCapture(10000));
            assertTrue(receipt.contains("\"scaledTicks\":[10000]"));
        });
    }

    @Test void semanticObservationMatchesTheExistingScaledLongWithoutClockRead() {
        withTrace(() -> {
            GraphicsAuditHandFoilTiming.beginFrame();
            GraphicsAuditHandFoilTiming.observeSemanticClock(2500,.5,.5f);
            assertTrue(GraphicsAuditHandFoilTiming.readyForCapture(10000));
            assertTrue(GraphicsAuditHandFoilTiming.snapshot().contains("\"scaledTicks\":[10000]"));
            GraphicsAuditHandFoilTiming.beginFrame();
            GraphicsAuditHandFoilTiming.observeSemanticClock(2629,.5,.5f);
            assertFalse(GraphicsAuditHandFoilTiming.readyForCapture(10000));
        });
    }

    @Test void rejectsAmbiguousMultipleSamplesAndWrongTextureScale() {
        withTrace(() -> {
            GraphicsAuditHandFoilTiming.beginFrame();
            GraphicsAuditHandFoilTiming.beginHand();
            GraphicsAuditHandFoilTiming.observeScaledTicks(10000,.5f);
            GraphicsAuditHandFoilTiming.endHand();
            assertFalse(GraphicsAuditHandFoilTiming.readyForCapture(10000));
            GraphicsAuditHandFoilTiming.beginFrame();
            GraphicsAuditHandFoilTiming.observeSemanticClock(2500,.5,.5f);
            GraphicsAuditHandFoilTiming.observeSemanticClock(2500,.5,.5f);
            assertFalse(GraphicsAuditHandFoilTiming.readyForCapture(10000));
        });
    }

    @Test void rejectsMalformedInputsAndBoundsReceiptSize() {
        withTrace(() -> {
            GraphicsAuditHandFoilTiming.beginFrame();
            GraphicsAuditHandFoilTiming.observeSemanticClock(2500,Double.NaN,.5f);
            assertFalse(GraphicsAuditHandFoilTiming.readyForCapture(10000));
            GraphicsAuditHandFoilTiming.beginFrame();
            for (int i=0;i<1000;i++) GraphicsAuditHandFoilTiming.observeSemanticClock(2500,.5,.5f);
            assertTrue(GraphicsAuditHandFoilTiming.snapshot().length()<256);
            assertTrue(GraphicsAuditHandFoilTiming.snapshot().contains("\"complete\":false"));
        });
    }

    @Test void leakedScopeInvalidatesNextFrameAndCanRecover() {
        withTrace(() -> {
            GraphicsAuditHandFoilTiming.beginFrame();
            GraphicsAuditHandFoilTiming.beginHand();
            GraphicsAuditHandFoilTiming.beginFrame();
            GraphicsAuditHandFoilTiming.observeSemanticClock(2500,.5,.5f);
            assertFalse(GraphicsAuditHandFoilTiming.readyForCapture(10000));
            GraphicsAuditHandFoilTiming.beginFrame();
            GraphicsAuditHandFoilTiming.observeSemanticClock(2500,.5,.5f);
            assertTrue(GraphicsAuditHandFoilTiming.readyForCapture(10000));
        });
    }
}
