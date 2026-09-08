package net.minecraft.client.dev;

import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class GraphicsAuditWorldGuiScaleTest {
    @Test void worldCycleMustFinishBeforeRustFinalOutputCaptureIsArmed() throws Exception {
        String source = java.nio.file.Files.readString(java.nio.file.Path.of(
            "src/main/java/net/minecraft/client/dev/DeterministicCameraCapture.java"));
        int cycle = source.indexOf("if (!GraphicsAuditWorldGuiScale.prepareWorldCapture");
        int arm = source.indexOf("wholeFrameAttachmentCaptureArmed = true;");
        assertTrue(cycle > 0 && arm > cycle,
            "post-present final-output acknowledgement bypasses late ordinary screenshot hooks");
        assertTrue(source.substring(cycle, arm).contains("resetWholeFrameAttachmentCaptureState();"));
    }
    @Test void requiresDistinctPresentedFramesAtEveryObservedScale() {
        var sequence = new GraphicsAuditWorldGuiScale.Sequence();
        assertNull(sequence.afterPresentation(1, 2, 2));
        assertNull(sequence.afterPresentation(1, 2, 2));
        assertEquals(3, sequence.afterPresentation(2, 2, 2));
        assertNull(sequence.afterPresentation(3, 3, 2));
        assertNull(sequence.afterPresentation(4, 3, 3));
        assertEquals(2, sequence.afterPresentation(5, 3, 3));
        assertNull(sequence.afterPresentation(6, 2, 3));
        assertNull(sequence.afterPresentation(7, 2, 2));
        assertFalse(sequence.complete());
        assertNull(sequence.afterPresentation(8, 2, 2));
        assertTrue(sequence.complete());
        assertEquals(6, sequence.receipt().getAsJsonArray("observations").size());
        assertNull(sequence.afterPresentation(9, 2, 2));
        assertEquals(6, sequence.receipt().getAsJsonArray("observations").size());
    }
    @Test void rejectsWrongInitialScaleAndNonMonotonicFrames() {
        assertThrows(IllegalStateException.class,
            () -> new GraphicsAuditWorldGuiScale.Sequence().afterPresentation(1, 3, 3));
        var sequence = new GraphicsAuditWorldGuiScale.Sequence();
        sequence.afterPresentation(2, 2, 2);
        assertThrows(IllegalStateException.class, () -> sequence.afterPresentation(1, 2, 2));
    }
    @Test void stalledApplicationCannotComplete() {
        var sequence = new GraphicsAuditWorldGuiScale.Sequence();
        sequence.afterPresentation(1, 2, 2);
        sequence.afterPresentation(2, 2, 2);
        for (int frame = 3; frame <= 120; frame++) sequence.afterPresentation(frame, 3, 2);
        assertFalse(sequence.complete());
        assertThrows(IllegalStateException.class, () -> sequence.afterPresentation(121, 3, 2));
    }
}
