package net.minecraft.client.dev;

import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.LinkedHashMap;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class GraphicsAuditSurfaceCaptureTest {
    @Test void selectedFrameFreezesAllSurfacesWithoutReadingFutureObservations() throws Exception {
        var saved = new LinkedHashMap<Field, Object>();
        try {
            Class<?>[] types = {GraphicsAuditFlowingWaterFixture.class,
                GraphicsAuditLavaFixture.class, GraphicsAuditMixedFluidFixture.class};
            for (var type : types) {
                String suffix = type == GraphicsAuditMixedFluidFixture.class ? "BottomSurface" : "Surface";
                for (String prefix : new String[] {"latest", "captured"}) {
                    Field field = type.getDeclaredField(prefix + suffix);
                    field.setAccessible(true);
                    saved.put(field, field.get(null));
                    field.set(null, prefix.equals("latest") ? "[[12.5,24.5]]" : "null");
                }
            }
            Field particles = GraphicsAuditLavaFixture.class.getDeclaredField("capturedParticles");
            particles.setAccessible(true);
            saved.put(particles, particles.get(null));
            DeterministicCameraCapture.captureSurfaceObservations();
            for (var field : saved.keySet()) {
                if (field.getName().startsWith("latest")) field.set(null, "[[99,99]]");
            }
            assertEquals("[[12.5,24.5]]", GraphicsAuditFlowingWaterFixture.capturedSurface());
            assertEquals("[[12.5,24.5]]", GraphicsAuditLavaFixture.capturedSurface());
            assertEquals("[[12.5,24.5]]", GraphicsAuditMixedFluidFixture.capturedBottomSurface());
        } finally {
            for (var entry : saved.entrySet()) entry.getKey().set(null, entry.getValue());
        }
    }

    @Test void bothCapturePathsFreezeSurfacesButMetadataDoesNotRefreshThem() throws Exception {
        String source = Files.readString(Path.of("src/main/java/net/minecraft/client/dev/DeterministicCameraCapture.java"));
        String claim = source.substring(source.indexOf("public static long claimWholeFrameAttachmentCaptureRenderedFrameIndex()"),
            source.indexOf("public static long currentCaptureCorrelationRenderedFrameIndex()"));
        assertTrue(claim.indexOf("captureSurfaceObservations();") > claim.indexOf("GraphicsAuditGuiFoilTiming.readyForCapture()"));
        assertTrue(claim.indexOf("captureSurfaceObservations();") < claim.indexOf("return wholeFrameAttachmentCaptureDeterministicFrame;"));
        assertTrue(source.contains("blockAnimationAtCapture = GraphicsAuditBlockDisplayFixture.animationObservation(minecraft);\n\t\tcaptureSurfaceObservations();"));
        assertEquals(2, source.split("captureSurfaceObservations\\(\\);", -1).length - 1);
        String observations = source.substring(source.indexOf("static void captureSurfaceObservations()"),
            source.indexOf("private static void requestCurrentPoseScreenshot("));
        assertTrue(observations.contains("StaticTerrainParityDiagnostics.recordAppearanceSourceAtCapture();"),
            "CPU mesh evidence must be observed for Rust final-output captures as well as ordinary screenshots");
        assertEquals(1, source.split("recordAppearanceSourceAtCapture\\(\\);", -1).length - 1);
    }
}
