package net.minecraft.client.dev;

import org.joml.Matrix4fc;

/** Bounded CPU-only observations. Never consumed by a rendering route. */
public final class GraphicsAuditTerrainInputs {
    private static final Snapshot LATEST = new Snapshot();
    private GraphicsAuditTerrainInputs() {}

    private static boolean enabled() {
        return System.getProperty("mattmc.dev.staticTerrainParityDiagnostics.appearanceTraceSection") != null;
    }

    public static void matrix(String role, Matrix4fc matrix) {
        if (enabled() && matrix != null) LATEST.observe(role, matrix.get(new float[16]));
    }

    public static void shrink(float x, float y) {
        if (enabled()) LATEST.observe("shrink", new float[]{x,y});
    }

    public static String json() { return LATEST.json(); }

    static final class Snapshot {
        private int[] projection, view, shrink;
        synchronized void observe(String role, float[] values) {
            int length = switch(role) {
                case "projection", "view" -> 16;
                case "shrink" -> 2;
                default -> 0;
            };
            if (values == null || length == 0 || values.length != length) return;
            int[] bits = new int[length];
            for (int i=0;i<length;i++) {
                if (!Float.isFinite(values[i])) return;
                bits[i] = Float.floatToRawIntBits(values[i]);
            }
            switch(role) {
                case "projection" -> projection = bits;
                case "view" -> view = bits;
                case "shrink" -> shrink = bits;
                default -> {}
            }
        }
        synchronized String json() {
            return "{\"projectionBits\":" + words(projection) + ",\"viewBits\":" + words(view)
                + ",\"shrinkBits\":" + words(shrink) + "}";
        }
        private static String words(int[] values) {
            if (values == null) return "null";
            StringBuilder out = new StringBuilder("[");
            for (int i=0;i<values.length;i++) {
                if (i>0) out.append(',');
                out.append(values[i]);
            }
            return out.append(']').toString();
        }
    }
}
