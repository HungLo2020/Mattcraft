package net.minecraft.client.dev;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import net.minecraft.client.Minecraft;

/** Capture-only normal window resize requests with observed presentation receipts. */
public final class GraphicsAuditWorldResize {
    private static final boolean ENABLED =
        Boolean.getBoolean("mattmc.dev.deterministicCameraCapture.windowResizeCycle");
    private static final Sequence ACTIVE = new Sequence(Integer.getInteger(
        "mattmc.dev.deterministicCameraCapture.windowResizeCaptureStage", 3));
    private GraphicsAuditWorldResize() {}

    public static final class Sequence {
        private static final int[][] SIZES = {{1280,720}, {640,480}, {1600,900}, {1280,720}};
        private final JsonArray observations = new JsonArray();
        private long lastFrame = Long.MIN_VALUE;
        private int phase;
        private int presentations;
        private int attempts;
        private final int captureStage;

        public Sequence() { this(3); }
        public Sequence(int captureStage) {
            if (captureStage < 1 || captureStage > 3)
                throw new IllegalArgumentException("Invalid window resize capture stage");
            this.captureStage = captureStage;
        }

        public int[] afterPresentation(long frame, int width, int height, int scale) {
            return afterPresentation(frame, width, height, scale, 32, 64);
        }

        public int[] afterPresentation(long frame, int width, int height, int scale, int x, int y) {
            if (frame <= 0 || frame < lastFrame)
                throw new IllegalStateException("Invalid window resize presentation identity");
            if (frame == lastFrame || complete()) return null;
            lastFrame = frame;
            if (++attempts > 180) throw new IllegalStateException("World window resize failed to settle");
            int[] expected = SIZES[phase];
            if (attempts == 1 && (width != 1280 || height != 720 || scale != 2))
                throw new IllegalStateException("World resize requires initial1280x720 scale2");
            if (width != expected[0] || height != expected[1] || scale != 2
                    || (phase > 0 && (x != 32 || y != 64))) {
                presentations = 0;
                return null;
            }
            var observation = new JsonObject();
            observation.addProperty("frame", frame);
            observation.addProperty("phase", phase);
            observation.addProperty("width", width);
            observation.addProperty("height", height);
            observation.addProperty("scale", scale);
            observation.addProperty("x", x);
            observation.addProperty("y", y);
            observations.add(observation);
            if (++presentations < 2) return null;
            presentations = 0;
            phase++;
            return complete() ? null : SIZES[phase].clone();
        }

        public boolean complete() { return phase == captureStage + 1; }
        public JsonObject receipt() {
            var receipt = new JsonObject();
            receipt.addProperty("schema", "world-window-resize-recovery-v1");
            receipt.addProperty("complete", complete());
            receipt.addProperty("captureStage", captureStage);
            receipt.addProperty("windowPlacement", "client-32-64-v1");
            receipt.add("observations", observations.deepCopy());
            return receipt;
        }
    }

    public static boolean prepareWorldCapture(Minecraft minecraft, long frame) {
        if (!ENABLED) return true;
        var window = minecraft.getWindow();
        int[] target = ACTIVE.afterPresentation(frame, window.getWidth(), window.getHeight(),
            window.getGuiScale(), window.getX(), window.getY());
        if (target != null) {
            window.setWindowed(target[0], target[1]);
            // Equivalent test input in both repositories. Keep the large
            // drawable clear of desktop panels before presenting/capturing.
            org.lwjgl.glfw.GLFW.glfwSetWindowPos(window.handle(), 32, 64);
        }
        return ACTIVE.complete();
    }

    public static String worldReceipt() {
        return ENABLED ? ACTIVE.receipt().toString() : "null";
    }
}
