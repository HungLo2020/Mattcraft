package net.minecraft.client.dev;

import java.util.ArrayList;
import java.util.List;

/** Opt-in observations only: never provides clocks, matrices or renderer inputs. */
public final class GraphicsAuditGuiFoilTiming {
    private static final List<String> samples = new ArrayList<>();
    private static final ThreadLocal<int[]> slot = new ThreadLocal<>();
    private static boolean complete = true;
    private static long frameSequence;
    private static final List<Long> phaseTicks = new ArrayList<>();
    private static long waitStarted;

    public static boolean movingRequested() {
        return System.getProperty("mattmc.dev.graphicsAuditGuiItemFoilPhase") != null;
    }

    static boolean phaseMatches(long ticks, int target) {
        return ticks >= 0 && target >= 0 && target < 330000
            && Math.floorMod(ticks - target, 330000L) <= 512L;
    }

    /** Only delays capture selection; never substitutes a render clock. */
    public static synchronized boolean readyForCapture() {
        if (!movingRequested()) return true;
        int target = Integer.parseInt(System.getProperty("mattmc.dev.graphicsAuditGuiItemFoilPhase"));
        if (!enabled() || target < 0 || target >= 330000)
            throw new IllegalStateException("invalid animated foil diagnostic configuration");
        if (waitStarted == 0) waitStarted = System.nanoTime();
        if (System.nanoTime() - waitStarted > 110_000_000_000L)
            throw new IllegalStateException("timed out observing the requested natural foil phase");
        return complete && slot.get() == null && phaseTicks.size() == 8
            && phaseTicks.stream().allMatch(ticks -> phaseMatches(ticks, target));
    }

    private static boolean enabled() { return Boolean.getBoolean("mattmc.dev.guiItemRasterTrace"); }

    public static synchronized void beginFrame() {
        if (!enabled()) return;
        samples.clear();
        phaseTicks.clear();
        complete = slot.get() == null;
        slot.remove();
        frameSequence++;
    }

    public static void beginItem(int x, int y) {
        if (!enabled()) return;
        if (slot.get() != null) invalidate();
        slot.set(new int[]{x,y});
    }

    public static void endItem() { slot.remove(); }

    // Frozen observes the existing scaled long BEFORE its two remainder operations.
    // This avoids adding a second clock read or changing expression evaluation.
    public static void observeScaledTicks(long ticks, float textureScale) {
        if (!enabled()) return;
        int[] position = slot.get();
        if (position == null) return; // world glint is outside this GUI diagnostic
        if (ticks < 0 || textureScale != 8.0F) { invalidate(); return; }
        append(position[0], position[1], "\"scaledTicks\":" + ticks);
        observePhase(ticks);
    }

    // Current observes exactly the immutable semantic payload sent to Rust.
    public static void observeSemanticClock(int x, int y, long clock, double speed, float strength) {
        if (!enabled()) return;
        if (clock < 0 || !Double.isFinite(speed) || speed < 0 || speed > 1
            || !Float.isFinite(strength) || strength < 0 || strength > 1) {
            invalidate(); return;
        }
        append(x,y,"\"clockMillis\":" + clock + ",\"speed\":" + speed + ",\"strength\":" + strength);
        // Diagnostic-only normalization; this value never feeds a renderer.
        observePhase((long)(clock * speed * 8.0));
    }

    private static synchronized void observePhase(long ticks) {
        if (phaseTicks.size() < 64) phaseTicks.add(ticks);
    }

    private static synchronized void invalidate() { complete = false; }

    private static synchronized void append(int x, int y, String values) {
        if (samples.size() >= 64) { complete = false; return; }
        samples.add("{\"x\":" + x + ",\"y\":" + y + "," + values + "}");
    }

    /** Immutable JSON snapshot; later frames cannot rewrite captured evidence. */
    public static synchronized String snapshot() {
        return "{\"enabled\":" + enabled() + ",\"complete\":" + (complete && slot.get() == null)
            + ",\"frameSequence\":" + frameSequence + ",\"samples\":[" + String.join(",",samples) + "]}";
    }

    static synchronized void resetForTest() {
        samples.clear(); slot.remove(); complete = true; frameSequence = 0;
        phaseTicks.clear(); waitStarted = 0;
    }
}
