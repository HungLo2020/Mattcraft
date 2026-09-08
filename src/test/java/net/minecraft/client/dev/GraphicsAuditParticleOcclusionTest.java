package net.minecraft.client.dev;
import java.util.ArrayList;
import net.minecraft.world.phys.Vec3;
import org.joml.Matrix4f;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;
class GraphicsAuditParticleOcclusionTest {
    private GraphicsAuditParticleOcclusion observation() {
        var points = new ArrayList<Vec3>();
        for (double x : new double[] {-0.01, 0, 0.01})
            for (double y : new double[] {-0.01, 0, 0.01}) points.add(new Vec3(x, y, 0));
        return new GraphicsAuditParticleOcclusion(new Matrix4f(), 100, 100, points, Vec3.ZERO);
    }
    @Test void incompleteAndUnknownCollectionsCannotAdmitCapture() {
        var state = observation();
        assertFalse(state.ready());
        state.begin();
        assertFalse(state.ready());
        state.end();
        assertTrue(state.ready());
        state.begin();
        state.unknownGroup();
        state.end();
        assertFalse(state.ready());
    }
    @Test void positiveAndNegativeQuadsObstructTheSamePredeterminedSamples() {
        for (float size : new float[] {0.1F, -0.1F}) {
            var state = observation();
            state.begin();
            state.quad(0, 0, 0, 0, 0, 0, 1, size, -1);
            state.end();
            assertFalse(state.ready());
            state.begin();
            state.quad(0.8F, 0, 0, 0, 0, 0, 1, size, -1);
            state.end();
            assertTrue(state.ready());
        }
    }
    @Test void transparentOrZeroAreaQuadsDoNotHideTheSurface() {
        var state = observation();
        state.begin();
        state.quad(0, 0, 0, 0, 0, 0, 1, 0.1F, 0);
        state.quad(0, 0, 0, 0, 0, 0, 1, 0, -1);
        state.end();
        assertTrue(state.ready());
        assertTrue(state.receipt().contains("\"quads\":2"));
    }
    @Test void nonfiniteProjectionFailsClosed() {
        var state = observation();
        state.begin();
        state.quad(0, 0, 0, Float.NaN, 0, 0, 1, 0.1F, -1);
        state.end();
        assertFalse(state.ready());
    }
}
