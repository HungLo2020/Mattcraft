package net.minecraft.client.dev;

import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;
import static net.minecraft.client.dev.GraphicsAuditResourceReload.Sequence.Action.*;

class GraphicsAuditResourceReloadTest {
    @Test
    void worldReloadMustCompleteBeforeFinalOutputCaptureArming() throws Exception {
        String source = java.nio.file.Files.readString(java.nio.file.Path.of(
            "src/main/java/net/minecraft/client/dev/DeterministicCameraCapture.java"));
        int reload = source.indexOf("if (!GraphicsAuditResourceReload.prepareWorldCapture");
        int arm = source.indexOf("wholeFrameAttachmentCaptureArmed = true;");
        assertTrue(reload > 0 && arm > reload,
            "the final-output acknowledgement must not bypass the real reload");
        assertTrue(source.substring(reload, arm).contains("resetWholeFrameAttachmentCaptureState();"));
    }
    @Test
    void replacementRequiresAnActuallySelectedPackAndPreservesRemainingOrder() {
        var selected = java.util.List.of("vanilla", "file/a", "file/b");
        assertEquals(java.util.List.of("vanilla", "file/a"),
            GraphicsAuditResourceReload.withoutSelectedPack(selected, "file/b"));
        assertEquals(java.util.List.of("vanilla", "file/a", "file/b"), selected);
        assertThrows(IllegalArgumentException.class,
            () -> GraphicsAuditResourceReload.withoutSelectedPack(selected, "file/missing"));
        assertThrows(IllegalArgumentException.class,
            () -> GraphicsAuditResourceReload.withoutSelectedPack(java.util.List.of("file/b"), "file/b"));
    }

    @Test
    void worldReloadCallsRealOperationOnceAndRequiresDistinctCompletedFrames() {
        var sequence = new GraphicsAuditResourceReload.WorldSequence();
        var future = new java.util.concurrent.CompletableFuture<Void>();
        var calls = new java.util.concurrent.atomic.AtomicInteger();
        java.util.function.Supplier<java.util.concurrent.CompletableFuture<Void>> begin = () -> {
            calls.incrementAndGet();
            return future;
        };
        assertFalse(sequence.afterPresentation(10, false, begin));
        assertFalse(sequence.afterPresentation(11, false, begin));
        assertFalse(sequence.receipt().get("futureComplete").getAsBoolean());
        future.complete(null);
        assertFalse(sequence.afterPresentation(12, false, begin));
        assertFalse(sequence.afterPresentation(12, false, begin));
        assertFalse(sequence.afterPresentation(13, true, begin));
        assertFalse(sequence.afterPresentation(14, false, begin));
        assertTrue(sequence.afterPresentation(15, false, begin));
        assertTrue(sequence.receipt().get("complete").getAsBoolean());
        assertEquals(2, sequence.receipt().get("presentations").getAsInt());
        assertEquals(1, calls.get());
    }

    @Test
    void failedWorldReloadCannotBecomeSuccessfulEvidence() {
        var sequence = new GraphicsAuditResourceReload.WorldSequence();
        var future = java.util.concurrent.CompletableFuture.<Void>failedFuture(
            new IllegalStateException("fixture reload failure"));
        assertFalse(sequence.afterPresentation(1, false, () -> future));
        assertThrows(java.util.concurrent.CompletionException.class,
            () -> sequence.afterPresentation(2, false, () -> future));
        assertFalse(sequence.receipt().get("futureComplete").getAsBoolean());
        assertFalse(sequence.receipt().get("complete").getAsBoolean());
    }

    @Test
    void reloadCompletionAndTwoUnobstructedPresentationsAreRequired() {
        var sequence = new GraphicsAuditResourceReload.Sequence();
        assertEquals(RELOAD, sequence.afterPresentation(true, false));
        assertFalse(sequence.complete());
        assertEquals(WAIT, sequence.afterPresentation(false, false));
        assertEquals(WAIT, sequence.afterPresentation(true, true));
        assertEquals(WAIT, sequence.afterPresentation(true, false));
        assertEquals(WAIT, sequence.afterPresentation(true, true));
        assertEquals(WAIT, sequence.afterPresentation(true, false));
        assertFalse(sequence.complete());
        assertEquals(CAPTURE, sequence.afterPresentation(true, false));
        assertTrue(sequence.complete());
    }
}
