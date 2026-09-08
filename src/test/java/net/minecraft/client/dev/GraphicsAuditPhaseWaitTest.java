package net.minecraft.client.dev;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;
class GraphicsAuditPhaseWaitTest {
    @Test void normalItemReadbackClaimsOnlyAnEligibleObservedAnimationFrame() throws Exception {
        String source=java.nio.file.Files.readString(java.nio.file.Path.of(
            "src/main/java/net/minecraft/client/dev/DeterministicCameraCapture.java"));
        int claim=source.indexOf("public static long claimWholeFrameAttachmentCaptureRenderedFrameIndex()");
        int issued=source.indexOf("wholeFrameAttachmentCaptureRequestIssued = true;",claim);
        String beforeClaim=source.substring(claim,issued);
        assertTrue(beforeClaim.contains("GraphicsAuditBlockDisplayFixture.readyForCapture(minecraft)"));
        assertTrue(beforeClaim.contains("blockAnimationAtCapture = GraphicsAuditBlockDisplayFixture.animationObservation(minecraft)"));
        assertTrue(beforeClaim.contains("return -1L"));
    }
    @Test void animationWaitCountsRealProgressRatherThanDuplicateRenderFrames() {
        var time = new java.util.concurrent.atomic.AtomicLong();
        var wait = new GraphicsAuditPhaseWait(time::get);
        for (int tick = 0; tick < 152; tick++) {
            for (int render = 0; render < 8; render++)
                assertFalse(wait.observeAnimation(false, tick % 76, 76));
            time.addAndGet(50_000_000L);
        }
        assertTrue(wait.observeAnimation(true, 0, 76));
        // Readiness resets the complete budget, not just the frame counter.
        for (int tick = 0; tick < 400; tick++)
            assertFalse(wait.observeAnimation(false, tick % 76, 76));
        assertTrue(wait.observeAnimation(true, 0, 76));
        // Slow rendering may observe several ordinary catch-up ticks at once.
        for (int render = 0; render < 100; render++)
            assertFalse(wait.observeAnimation(false, (render * 8) % 76, 76));
        assertTrue(wait.observeAnimation(true, 6, 76));
    }
    @Test void stalledAnimationRemainsBoundedAndInvalidCyclesAreRejected() {
        var time = new java.util.concurrent.atomic.AtomicLong();
        var wait = new GraphicsAuditPhaseWait(time::get);
        assertFalse(wait.observeAnimation(false, 0, 76));
        time.set(45_000_000_000L);
        assertThrows(IllegalStateException.class, () -> wait.observeAnimation(false, 0, 76));
        var frameBound = new GraphicsAuditPhaseWait(() -> 0);
        for (int i = 0; i < 16384; i++) assertFalse(frameBound.observeAnimation(false, 0, 76));
        assertThrows(IllegalStateException.class, () -> frameBound.observeAnimation(false, 0, 76));
        assertThrows(IllegalStateException.class, () -> wait.observeAnimation(true, 0, 0));
        assertThrows(IllegalStateException.class, () -> wait.observeAnimation(true, 76, 76));
    }
    @Test void waitsAreBoundedAndReadinessResetsTheBudget() {
        var wait = new GraphicsAuditPhaseWait();
        for (int i = 0; i < 512; i++) assertFalse(wait.observe(false));
        assertTrue(wait.observe(true));
        for (int i = 0; i < 512; i++) assertFalse(wait.observe(false));
        assertThrows(IllegalStateException.class, () -> wait.observe(false));
    }
    @Test void cycleBoundaryRequiresARealPositiveResourceTick() {
        assertFalse(GraphicsAuditPhaseWait.cycleBoundary(0, 24));
        assertFalse(GraphicsAuditPhaseWait.cycleBoundary(23, 24));
        assertTrue(GraphicsAuditPhaseWait.cycleBoundary(24, 24));
        assertTrue(GraphicsAuditPhaseWait.cycleBoundary(48, 24));
        assertFalse(GraphicsAuditPhaseWait.cycleBoundary(24, 0));
        assertFalse(GraphicsAuditPhaseWait.cycleBoundary(-24, 24));
    }
    @Test void capturePhaseUsesDeclaredDurationWithoutAdvancingTicks() {
        for (long tick = 1; tick <= 72; tick++) {
            for (long phase = 0; phase < 24; phase++) {
                assertEquals(tick % 24 == phase, GraphicsAuditPhaseWait.phaseMatches(tick, 24, phase));
            }
        }
        assertFalse(GraphicsAuditPhaseWait.phaseMatches(0, 24, 0));
        assertFalse(GraphicsAuditPhaseWait.phaseMatches(24, 24, 24));
        assertFalse(GraphicsAuditPhaseWait.phaseMatches(24, 24, -1));
        assertFalse(GraphicsAuditPhaseWait.phaseMatches(24, 0, 0));
    }
    @Test void invalidConfiguredPhaseFailsInsteadOfSilentlyCapturingAnotherPhase() {
        String key = "mattmc.dev.graphicsAuditMagmaCapturePhase";
        String previous = System.getProperty(key);
        try {
            System.clearProperty(key);
            assertEquals(0, GraphicsAuditPhaseWait.requestedPhase(24));
            System.setProperty(key, "3");
            assertEquals(3, GraphicsAuditPhaseWait.requestedPhase(24));
            for (String invalid : new String[]{"-1", "24", "bad"}) {
                System.setProperty(key, invalid);
                assertThrows(IllegalStateException.class, () -> GraphicsAuditPhaseWait.requestedPhase(24));
            }
        } finally {
            if (previous == null) System.clearProperty(key); else System.setProperty(key, previous);
        }
    }
}
