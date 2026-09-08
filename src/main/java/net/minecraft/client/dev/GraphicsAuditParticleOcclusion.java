package net.minecraft.client.dev;

import java.util.List;
import net.minecraft.world.phys.Vec3;
import org.joml.Matrix4f;
import org.joml.Matrix4fc;
import org.joml.Quaternionf;
import org.joml.Vector3f;
import org.joml.Vector4f;

/** Read-only conservative particle bounds for predetermined fluid capture samples. */
public final class GraphicsAuditParticleOcclusion {
    private final Matrix4f matrix;
    private final int width, height;
    private final double[][] samples;
    private boolean begun, complete;
    private int quads, blockers;
    public GraphicsAuditParticleOcclusion(Matrix4fc matrix, int width, int height,
            List<Vec3> points, Vec3 eye) {
        this.matrix = new Matrix4f(matrix);
        this.width = width;
        this.height = height;
        samples = new double[points.size()][2];
        for (int i = 0; i < points.size(); i++) {
            Vec3 p = points.get(i).subtract(eye);
            var clip = this.matrix.transform(new Vector4f((float)p.x, (float)p.y, (float)p.z, 1));
            samples[i] = project(clip);
        }
    }
    private double[] project(Vector4f clip) {
        if (!Float.isFinite(clip.w) || clip.w <= 0 || !Float.isFinite(clip.x) || !Float.isFinite(clip.y))
            return new double[] {Double.NaN, Double.NaN};
        return new double[] {(clip.x / clip.w + 1) * width * 0.5,
            (1 - clip.y / clip.w) * height * 0.5};
    }
    public void begin() { begun = true; complete = false; quads = 0; blockers = 0; }
    public void unknownGroup() { if (begun) blockers++; }
    public void end() { complete = begun; }
    public void quad(float x, float y, float z, float qx, float qy, float qz, float qw,
            float size, int argb) {
        if (!begun) return;
        quads++;
        if (size == 0 || argb >>> 24 == 0) return;
        var rotation = new Quaternionf(qx, qy, qz, qw);
        double minX = Double.POSITIVE_INFINITY, minY = minX;
        double maxX = Double.NEGATIVE_INFINITY, maxY = maxX;
        for (float cx : new float[] {-1, 1}) for (float cy : new float[] {-1, 1}) {
            var point = rotation.transform(new Vector3f(cx * size, cy * size, 0)).add(x, y, z);
            var pixel = project(matrix.transform(new Vector4f(point, 1)));
            if (!Double.isFinite(pixel[0]) || !Double.isFinite(pixel[1])) { blockers++; return; }
            minX = Math.min(minX, pixel[0]); maxX = Math.max(maxX, pixel[0]);
            minY = Math.min(minY, pixel[1]); maxY = Math.max(maxY, pixel[1]);
        }
        for (double[] sample : samples) {
            // Same fixed5x5 crop centers as the image gate, with a conservative
            // margin for raster coverage and serialized-coordinate rounding.
            // Transparent texels and depth may make a box
            // overly conservative; they can never excuse an obstructed sample.
            double sx = Math.rint(sample[0]), sy = Math.rint(sample[1]);
            if (minX <= sx + 4 && maxX >= sx - 4 && minY <= sy + 4 && maxY >= sy - 4) {
                blockers++;
                return;
            }
        }
    }
    public boolean ready() {
        if (!complete || samples.length != 9 || blockers != 0
                || width <= 0 || height <= 0 || width > 32768 || height > 32768) return false;
        for (double[] sample : samples)
            if (!Double.isFinite(sample[0]) || !Double.isFinite(sample[1])) return false;
        return true;
    }
    public String receipt() {
        return "{\"schema\":\"fluid-sample-particle-occlusion-v1\",\"complete\":" + complete
            + ",\"quads\":" + quads + ",\"blockers\":" + blockers + ",\"ready\":" + ready() + "}";
    }
}
