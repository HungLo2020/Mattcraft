package net.minecraft.client.dev;

import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class GraphicsAuditWorldResizeTest {
    @Test void resizedWindowMustReachTheUnoccludedCapturePosition() {
        var sequence = new GraphicsAuditWorldResize.Sequence(1);
        sequence.afterPresentation(1,1280,720,2,320,180);
        sequence.afterPresentation(2,1280,720,2,320,180);
        assertNull(sequence.afterPresentation(3,640,480,2,320,131));
        assertNull(sequence.afterPresentation(4,640,480,2,32,64));
        assertFalse(sequence.complete());
        assertNull(sequence.afterPresentation(5,640,480,2,32,64));
        assertTrue(sequence.complete());
        assertEquals(4,sequence.receipt().getAsJsonArray("observations").size());
    }
    @Test void intermediateCaptureRequiresTheEntireRequestedPrefix() {
        int[][] sizes = {{1280,720}, {640,480}, {1600,900}, {1280,720}};
        for (int stage = 1; stage <= 3; stage++) {
            var sequence = new GraphicsAuditWorldResize.Sequence(stage);
            long frame = 0;
            for (int phase = 0; phase <= stage; phase++) {
                assertNull(sequence.afterPresentation(++frame, sizes[phase][0], sizes[phase][1], 2));
                assertFalse(sequence.complete());
                int[] request = sequence.afterPresentation(++frame, sizes[phase][0], sizes[phase][1], 2);
                if (phase == stage) assertNull(request);
                else assertArrayEquals(sizes[phase+1], request);
            }
            assertTrue(sequence.complete());
            assertEquals(stage, sequence.receipt().get("captureStage").getAsInt());
            assertEquals(2*(stage+1), sequence.receipt().getAsJsonArray("observations").size());
        }
        assertThrows(IllegalArgumentException.class, () -> new GraphicsAuditWorldResize.Sequence(0));
        assertThrows(IllegalArgumentException.class, () -> new GraphicsAuditWorldResize.Sequence(4));
    }
    @Test void resizeRecoveryMustFinishBeforeFinalOutputCaptureArming() throws Exception {
        String source = java.nio.file.Files.readString(java.nio.file.Path.of(
            "src/main/java/net/minecraft/client/dev/DeterministicCameraCapture.java"));
        int resize = source.indexOf("if (!GraphicsAuditWorldResize.prepareWorldCapture");
        int arm = source.indexOf("wholeFrameAttachmentCaptureArmed = true;");
        assertTrue(resize > 0 && arm > resize);
        assertTrue(source.substring(resize, arm).contains("resetWholeFrameAttachmentCaptureState();"));
    }
    @Test void requiresDistinctPresentationsAtEveryRequestedSize() {
        var sequence = new GraphicsAuditWorldResize.Sequence();
        assertNull(sequence.afterPresentation(1, 1280, 720, 2));
        assertNull(sequence.afterPresentation(1, 1280, 720, 2));
        assertArrayEquals(new int[]{640,480}, sequence.afterPresentation(2, 1280, 720, 2));
        assertNull(sequence.afterPresentation(3, 1280, 720, 2));
        assertNull(sequence.afterPresentation(4, 640, 480, 2));
        assertArrayEquals(new int[]{1600,900}, sequence.afterPresentation(5, 640, 480, 2));
        assertNull(sequence.afterPresentation(6, 1600, 900, 2));
        assertArrayEquals(new int[]{1280,720}, sequence.afterPresentation(7, 1600, 900, 2));
        assertNull(sequence.afterPresentation(8, 1280, 720, 2));
        assertFalse(sequence.complete());
        assertNull(sequence.afterPresentation(9, 1280, 720, 2));
        assertTrue(sequence.complete());
        assertEquals(8, sequence.receipt().getAsJsonArray("observations").size());
    }
    @Test void wrongInitialExtentAndRegressingFramesReject() {
        assertThrows(IllegalStateException.class,
            () -> new GraphicsAuditWorldResize.Sequence().afterPresentation(1, 640, 480, 2));
        var sequence = new GraphicsAuditWorldResize.Sequence();
        sequence.afterPresentation(2, 1280, 720, 2);
        assertThrows(IllegalStateException.class, () -> sequence.afterPresentation(1, 1280, 720, 2));
    }
    @Test void missingResizeCannotBeAccepted() {
        var sequence = new GraphicsAuditWorldResize.Sequence();
        sequence.afterPresentation(1, 1280, 720, 2);
        sequence.afterPresentation(2, 1280, 720, 2);
        for (int frame = 3; frame <= 180; frame++) sequence.afterPresentation(frame, 1280, 720, 2);
        assertFalse(sequence.complete());
        assertThrows(IllegalStateException.class, () -> sequence.afterPresentation(181, 1280, 720, 2));
    }
}
