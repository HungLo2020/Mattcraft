package net.minecraft.client.dev;

import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class GraphicsAuditTerrainInputsTest {
    @Test void observationsCopyExactBitsAndRejectMalformedUpdates() {
        var inputs = new GraphicsAuditTerrainInputs.Snapshot();
        assertEquals("{\"projectionBits\":null,\"viewBits\":null,\"shrinkBits\":null}", inputs.json());
        float[] matrix = new org.joml.Matrix4f().get(new float[16]);
        matrix[1] = -0.0F;
        inputs.observe("projection",matrix);
        inputs.observe("view",matrix);
        inputs.observe("shrink",new float[]{0.125F,-0.0F});
        String before = inputs.json();
        assertTrue(before.contains("-2147483648"));
        matrix[0]=99;
        assertEquals(before,inputs.json(),"must not retain mutable matrix arrays");
        inputs.observe("projection",new float[15]);
        inputs.observe("view",new float[]{Float.NaN});
        inputs.observe("shrink",new float[]{Float.POSITIVE_INFINITY,0});
        inputs.observe("unknown",new float[16]);
        assertEquals(before,inputs.json(),"malformed diagnostics must not replace valid observations");
        for (int i=0;i<1000;i++) inputs.observe("projection",matrix);
        assertTrue(inputs.json().length()<500,"retains one fixed-size snapshot, not a frame history");
    }
}
