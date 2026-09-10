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
    private static final java.util.Set<Long> timedItems = new java.util.HashSet<>();
    private static final java.util.Map<Long,int[]> atlasWrites = new java.util.LinkedHashMap<>();
    private static final List<String> atlasAliases = new ArrayList<>();
    private static final java.util.Set<Long> reusedItems = new java.util.HashSet<>();

    private static long positionKey(int x,int y) { return ((long)x << 32) ^ Integer.toUnsignedLong(y); }

    /** Observe actual writes/reuses only; these values never control atlas allocation or drawing. */
    public static synchronized void observeAtlasWrite(int x,int y,int atlasX,int atlasY) {
        if (!enabled() || !timedItems.contains(positionKey(x,y))) return;
        if (atlasWrites.size() >= 64 || atlasWrites.putIfAbsent(positionKey(atlasX,atlasY),
                new int[]{x,y,atlasX,atlasY}) != null) complete=false;
    }

    public static synchronized void observeAtlasReuse(int x,int y,int atlasX,int atlasY) {
        if (!enabled()) return;
        int[] source=atlasWrites.get(positionKey(atlasX,atlasY));
        if (source==null) return; // Never invent evidence for a previous-frame or unobserved write.
        long target=positionKey(x,y);
        if (timedItems.contains(target) || !reusedItems.add(target)) { complete=false;return; }
        if (atlasAliases.size() >= 64) { complete=false;return; }
        atlasAliases.add("{\"x\":"+x+",\"y\":"+y+",\"sourceX\":"+source[0]+",\"sourceY\":"+source[1]
            +",\"atlasX\":"+atlasX+",\"atlasY\":"+atlasY+"}");
    }

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
            throw new IllegalStateException("timed out observing the requested natural foil phase: guiTicks="
                + phaseTicks + " hand=" + GraphicsAuditHandFoilTiming.snapshot());
        return complete && slot.get() == null && !phaseTicks.isEmpty()
            && phaseTicks.size() == timedItems.size() && timedItems.size()+reusedItems.size() == 8
            && GraphicsAuditHandFoilTiming.readyForCapture(target)
            && phaseTicks.stream().allMatch(ticks -> phaseMatches(ticks, target));
    }

    private static boolean enabled() { return Boolean.getBoolean("mattmc.dev.guiItemRasterTrace"); }

    public static synchronized void beginFrame() {
        if (!enabled()) return;
        samples.clear();
        timedItems.clear();atlasWrites.clear();atlasAliases.clear();reusedItems.clear();
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
        timedItems.add(positionKey(x,y));
    }

    /** Immutable JSON snapshot; later frames cannot rewrite captured evidence. */
    public static synchronized String snapshot() {
        return "{\"enabled\":" + enabled() + ",\"complete\":" + (complete && slot.get() == null)
            + ",\"frameSequence\":" + frameSequence + ",\"samples\":[" + String.join(",",samples)
            + "],\"atlasWrites\":[" + String.join(",",atlasWrites.values().stream()
                .map(p -> "{\"x\":"+p[0]+",\"y\":"+p[1]+",\"atlasX\":"+p[2]+",\"atlasY\":"+p[3]+"}").toList())
            + "],\"atlasAliases\":[" + String.join(",",atlasAliases) + "]" + (GraphicsAuditHandFoilTiming.enabled()
                ? ",\"hand\":" + GraphicsAuditHandFoilTiming.snapshot() : "") + "}";
    }

    static synchronized void resetForTest() {
        samples.clear(); slot.remove(); complete = true; frameSequence = 0;
        timedItems.clear();atlasWrites.clear();atlasAliases.clear();reusedItems.clear();
        phaseTicks.clear(); waitStarted = 0;
    }
}
