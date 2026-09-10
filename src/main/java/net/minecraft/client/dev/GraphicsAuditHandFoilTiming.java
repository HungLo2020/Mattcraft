package net.minecraft.client.dev;

import java.util.ArrayList;
import java.util.List;

/** Diagnostic observations only. Never supplies a clock or any rendering input. */
public final class GraphicsAuditHandFoilTiming {
    private static final List<Long> ticks = new ArrayList<>();
    private static boolean inHand;
    private static boolean valid = true;
    private static long frameSequence;

    private GraphicsAuditHandFoilTiming() {}

    public static boolean enabled() {
        return Boolean.getBoolean("mattmc.dev.handItemFoilTiming");
    }

    public static synchronized void beginFrame() {
        if (!enabled()) return;
        ticks.clear();
        valid = !inHand;
        inHand = false;
        frameSequence++;
    }

    public static synchronized void beginHand() {
        if (!enabled()) return;
        if (inHand) valid = false;
        inHand = true;
    }

    public static synchronized void endHand() {
        if (!enabled()) return;
        if (!inHand) valid = false;
        inHand = false;
    }

    /** Frozen observes the existing scaled long, without another clock read. */
    public static synchronized void observeScaledTicks(long value, float scale) {
        if (!enabled() || !inHand) return;
        if (value < 0 || scale != 8.0F) { valid = false; return; }
        append(value);
    }

    /** Current observes the immutable semantic hand payload sent to Rust. */
    public static synchronized void observeSemanticClock(long clock, double speed, float strength) {
        if (!enabled()) return;
        if (clock < 0 || !Double.isFinite(speed) || speed < 0 || speed > 1
            || !Float.isFinite(strength) || strength < 0 || strength > 1) {
            valid = false;
            return;
        }
        append((long)(clock * speed * 8.0));
    }

    private static void append(long value) {
        if (ticks.size() >= 16) { valid = false; return; }
        ticks.add(value);
    }

    /** Select an observed frame, never alter or stall the render clock. */
    public static synchronized boolean readyForCapture(int target) {
        if (!enabled()) return true;
        return valid && !inHand && ticks.size() == 1
            && GraphicsAuditGuiFoilTiming.phaseMatches(ticks.getFirst(), target);
    }

    public static synchronized String snapshot() {
        return "{\"enabled\":" + enabled() + ",\"complete\":" + (valid && !inHand)
            + ",\"frameSequence\":" + frameSequence + ",\"scaledTicks\":" + ticks + "}";
    }

    static synchronized void resetForTest() {
        ticks.clear(); inHand = false; valid = true; frameSequence = 0;
    }
}
