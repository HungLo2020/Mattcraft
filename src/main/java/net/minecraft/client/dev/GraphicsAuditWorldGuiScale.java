package net.minecraft.client.dev;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import net.minecraft.client.Minecraft;

/** Capture-only ordinary GUI setting changes; never reads or supplies GPU state. */
public final class GraphicsAuditWorldGuiScale {
    private static final boolean ENABLED =
        Boolean.getBoolean("mattmc.dev.deterministicCameraCapture.guiScaleCycle");
    private static final Sequence ACTIVE = new Sequence();
    private GraphicsAuditWorldGuiScale() {}

    public static final class Sequence {
        private final JsonArray observations = new JsonArray();
        private long lastFrame = Long.MIN_VALUE;
        private int phase;
        private int presentations;
        private int attempts;

        /** Returns a setting to apply, or null while observing the normal renderer. */
        public Integer afterPresentation(long frame, int setting, int actual) {
            if (frame <= 0 || frame < lastFrame)
                throw new IllegalStateException("Invalid GUI scale presentation identity");
            if (frame == lastFrame || complete()) return null;
            lastFrame = frame;
            if (++attempts > 120) throw new IllegalStateException("GUI scale cycle failed to settle");
            int expected = phase == 1 ? 3 : 2;
            if (attempts == 1 && (setting != 2 || actual != 2))
                throw new IllegalStateException("World GUI scale cycle requires initial scale2");
            if (setting != expected || actual != expected) {
                presentations = 0;
                return null;
            }
            var observation = new JsonObject();
            observation.addProperty("frame", frame);
            observation.addProperty("phase", phase);
            observation.addProperty("setting", setting);
            observation.addProperty("actual", actual);
            observations.add(observation);
            if (++presentations < 2) return null;
            presentations = 0;
            phase++;
            return phase == 1 ? Integer.valueOf(3) : phase == 2 ? Integer.valueOf(2) : null;
        }

        public boolean complete() { return phase == 3; }

        public JsonObject receipt() {
            var receipt = new JsonObject();
            receipt.addProperty("schema", "world-gui-scale-2-3-2-v1");
            receipt.addProperty("complete", complete());
            receipt.add("observations", observations.deepCopy());
            return receipt;
        }
    }

    public static boolean prepareWorldCapture(Minecraft minecraft, long frame) {
        if (!ENABLED) return true;
        Integer target = ACTIVE.afterPresentation(frame, minecraft.options.guiScale().get(),
            minecraft.getWindow().getGuiScale());
        if (target != null) {
            minecraft.options.guiScale().set(target);
            minecraft.resizeDisplay();
        }
        return ACTIVE.complete();
    }

    public static String worldReceipt() {
        return ENABLED ? ACTIVE.receipt().toString() : "null";
    }
}

