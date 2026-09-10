package net.vulkanic.bridge;

import java.lang.foreign.Arena;
import java.lang.foreign.ValueLayout;
import java.util.List;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;
import static net.vulkanic.bridge.VulkanicGalBridge.*;

final class ExperienceOrbInstanceEncodingTest {
    private static float[] identity() { return new org.joml.Matrix4f().get(new float[16]); }

    @Test
    void placementIsImmutableAndEncodedWithoutRasterPolicy() {
        float[] pose = identity();
        float[] rotation = {0,0,0,1};
        var orb = new WorldExperienceOrbInstanceRecord(7,2,1,pose,rotation,42);
        pose[12] = 900;
        rotation[3] = 0;
        orb.entityTransform()[12] = 999;
        orb.cameraOrientation()[3] = 0;
        try (var arena = Arena.ofConfined()) {
            var bytes = encodeExperienceOrbInstances(arena,List.of(orb),2);
            assertEquals(112,bytes.byteSize());
            assertEquals(112,bytes.get(ValueLayout.JAVA_INT,0));
            assertEquals(1,bytes.get(ValueLayout.JAVA_INT,4));
            assertEquals(7,bytes.get(ValueLayout.JAVA_LONG,8));
            assertEquals(2,bytes.get(ValueLayout.JAVA_LONG,16));
            assertArrayEquals(identity(),bytes.asSlice(24,64).toArray(ValueLayout.JAVA_FLOAT));
            assertArrayEquals(new float[]{0,0,0,1},bytes.asSlice(88,16).toArray(ValueLayout.JAVA_FLOAT));
            assertEquals(42,bytes.get(ValueLayout.JAVA_INT,104));
            assertEquals(0,bytes.get(ValueLayout.JAVA_INT,108));
        }
    }

    @Test
    void orderingAndFrameBoundsAreChecked() {
        var first = new WorldExperienceOrbInstanceRecord(7,2,1,identity(),new float[]{0,0,0,1},42);
        var second = new WorldExperienceOrbInstanceRecord(8,2,0,identity(),new float[]{0,0,0,1},43);
        try (var arena = Arena.ofConfined()) {
            assertEquals(224,encodeExperienceOrbInstances(arena,List.of(first,first),1).byteSize());
            assertThrows(IllegalArgumentException.class,()->encodeExperienceOrbInstances(arena,List.of(first,second),1));
            assertThrows(IllegalArgumentException.class,()->encodeExperienceOrbInstances(arena,List.of(first),0));
            assertThrows(IllegalArgumentException.class,()->encodeExperienceOrbInstances(arena,List.of(first),65_536));
        }
        assertThrows(IllegalArgumentException.class,()->new WorldExperienceOrbInstanceRecord(0,2,0,identity(),new float[]{0,0,0,1},42));
        assertThrows(IllegalArgumentException.class,()->new WorldExperienceOrbInstanceRecord(7,2,0,new float[3],new float[]{0,0,0,1},42));
    }
}
