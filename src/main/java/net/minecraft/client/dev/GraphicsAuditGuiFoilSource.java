package net.minecraft.client.dev;

import java.util.LinkedHashMap;
import java.util.Map;
import net.minecraft.client.renderer.block.model.BakedQuad;
import net.minecraft.core.Direction;

/** Bounded opt-in observations of actual GUI source quads, never renderer inputs. */
public final class GraphicsAuditGuiFoilSource {
    private static final Map<String, Sample> SAMPLES = new LinkedHashMap<>();
    private static boolean complete = true;

    public record Sample(String sprite, float[] positions, float[] atlasUvs) {
        public Sample {
            if (sprite == null || !sprite.matches("[a-z0-9_.:/-]+") || positions.length != 12 || atlasUvs.length != 8)
                throw new IllegalArgumentException("invalid foil diagnostic sample");
            positions = positions.clone(); atlasUvs = atlasUvs.clone();
            for (float value : positions) if (!Float.isFinite(value)) throw new IllegalArgumentException("non-finite source position");
            for (float value : atlasUvs) if (!Float.isFinite(value)) throw new IllegalArgumentException("non-finite source UV");
        }
        @Override public float[] positions() { return positions.clone(); }
        @Override public float[] atlasUvs() { return atlasUvs.clone(); }
    }

    public static void record(BakedQuad quad) {
        if (!Boolean.getBoolean("mattmc.dev.guiItemRasterTrace") || quad.direction() != Direction.SOUTH) return;
        float[] positions = new float[12], uv = new float[8];
        for (int i = 0; i < 4; i++) {
            positions[i*3] = quad.getX(i); positions[i*3+1] = quad.getY(i); positions[i*3+2] = quad.getZ(i);
            uv[i*2] = quad.getTexU(i); uv[i*2+1] = quad.getTexV(i);
        }
        observe(new Sample(quad.sprite().contents().name().toString(), positions, uv));
    }

    static synchronized void observe(Sample sample) {
        if (!SAMPLES.containsKey(sample.sprite()) && SAMPLES.size() >= 64) { complete = false; return; }
        // A fixture that reuses one sprite with differing geometry is ambiguous;
        // do not silently choose whichever source happened to be recorded last.
        Sample old = SAMPLES.get(sample.sprite());
        if (old != null && (!java.util.Arrays.equals(old.positions(), sample.positions())
            || !java.util.Arrays.equals(old.atlasUvs(), sample.atlasUvs()))) complete = false;
        SAMPLES.put(sample.sprite(), sample);
    }

    public static synchronized void appendJson(StringBuilder json) {
        json.append("{\"enabled\":").append(Boolean.getBoolean("mattmc.dev.guiItemRasterTrace"))
            .append(",\"complete\":").append(complete).append(",\"sources\":[");
        boolean first = true;
        for (Sample sample : SAMPLES.values()) {
            if (!first) json.append(',');
            first = false;
            json.append("{\"sprite\":\"").append(sample.sprite()).append("\",\"positions\":")
                .append(java.util.Arrays.toString(sample.positions())).append(",\"atlasUvs\":")
                .append(java.util.Arrays.toString(sample.atlasUvs())).append('}');
        }
        json.append("]}");
    }

    public static String json() { StringBuilder json = new StringBuilder(); appendJson(json); return json.toString(); }

    static synchronized void resetForTest() { SAMPLES.clear(); complete = true; }
}
