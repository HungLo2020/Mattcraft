package net.minecraft.client.dev;

import net.minecraft.client.renderer.entity.state.ExperienceOrbRenderState;
import net.minecraft.world.phys.Vec3;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class GraphicsAuditExperienceOrbFixtureTest {
    @Test void captureWaitsForAuthoritativeBlockAndSettledOpaqueSkylight() {
        assertTrue(GraphicsAuditExperienceOrbFixture.settledOccluder(true,true,false,0));
        assertFalse(GraphicsAuditExperienceOrbFixture.settledOccluder(false,true,false,0));
        assertFalse(GraphicsAuditExperienceOrbFixture.settledOccluder(true,false,false,0));
        assertFalse(GraphicsAuditExperienceOrbFixture.settledOccluder(true,true,true,0));
        for (int sky = 1; sky <= 15; sky++)
            assertFalse(GraphicsAuditExperienceOrbFixture.settledOccluder(true,true,false,sky));
    }

    @Test void occluderTargetsOnlyStationaryFixtureCell() {
        var position = new Vec3(147.80143035574073,101.0991813627766,529.155950217724);
        assertEquals(new net.minecraft.core.BlockPos(147,101,529),
            GraphicsAuditExperienceOrbFixture.checkedOccluderPosition(position, false));
        assertThrows(IllegalArgumentException.class, () ->
            GraphicsAuditExperienceOrbFixture.checkedOccluderPosition(position, true));
        assertThrows(IllegalArgumentException.class, () ->
            GraphicsAuditExperienceOrbFixture.checkedOccluderPosition(new Vec3(Double.NaN,0,0), false));
    }

    @Test void distanceSequenceKeepsViewingRayAndChangesWorldDistance() {
        Vec3 eye = new Vec3(10,20,30), initial = new Vec3(7,19,29);
        for (int index = 0; index < 5; index++) {
            assertEquals(initial, GraphicsAuditExperienceOrbFixture.positionForPose(eye, initial, index, false));
            assertEquals((3.0 + index) / 3.0, GraphicsAuditExperienceOrbFixture
                .positionForPose(eye, initial, index, true).distanceTo(eye) / initial.distanceTo(eye), 1e-12);
        }
        assertThrows(IllegalArgumentException.class, () ->
            GraphicsAuditExperienceOrbFixture.positionForPose(eye, initial, 5, true));
    }

    @Test void terminalCameraRestoreIsNotASixthAnimationSample() {
        for (int index = 0; index < 5; index++) assertTrue(GraphicsAuditExperienceOrbFixture.isCapturePose(index));
        assertFalse(GraphicsAuditExperienceOrbFixture.isCapturePose(5));
        assertThrows(IllegalArgumentException.class, () -> GraphicsAuditExperienceOrbFixture.isCapturePose(-1));
        assertThrows(IllegalArgumentException.class, () -> GraphicsAuditExperienceOrbFixture.isCapturePose(6));
    }

    @Test void animationUsesPoseIndexNotWallClockAndRejectsInvalidInputs() {
        for (int index = 0; index < 5; index++) {
            assertEquals(4 + 2 * index, GraphicsAuditExperienceOrbFixture.ageForPose("4", "2", index));
            assertEquals(4, GraphicsAuditExperienceOrbFixture.ageForPose("4", "0", index));
        }
        assertThrows(IllegalArgumentException.class, () -> GraphicsAuditExperienceOrbFixture.ageForPose("4", "2", -1));
        assertThrows(IllegalArgumentException.class, () -> GraphicsAuditExperienceOrbFixture.ageForPose("4", "2", 5));
        assertThrows(IllegalArgumentException.class, () -> GraphicsAuditExperienceOrbFixture.ageForPose("19999", "1", 4));
        assertThrows(IllegalArgumentException.class, () -> GraphicsAuditExperienceOrbFixture.ageForPose("4", "NaN", 0));
    }

    @Test void iconInputsCoverEveryVanillaCellAndRejectOutOfRange() {
        int[] values = {1,3,7,17,37,73,149,307,617,1237,2477};
        for (int icon = 0; icon < values.length; icon++)
            assertEquals(values[icon], GraphicsAuditExperienceOrbFixture.valueForIcon(icon));
        assertThrows(IllegalArgumentException.class, () -> GraphicsAuditExperienceOrbFixture.valueForIcon(-1));
        assertThrows(IllegalArgumentException.class, () -> GraphicsAuditExperienceOrbFixture.valueForIcon(11));
    }

    @Test void fixtureTimingIsFiniteAndBounded() {
        assertEquals(4.25f, GraphicsAuditExperienceOrbFixture.checkedAge("4.25"));
        for (String value : new String[]{"NaN","Infinity","-1","20001","invalid"})
            assertThrows(IllegalArgumentException.class, () -> GraphicsAuditExperienceOrbFixture.checkedAge(value));
    }

    @Test void cameraPlacementIsDeterministicAndOffTheCrosshair() {
        Vec3 eye = new Vec3(10,20,30);
        Vec3 p = GraphicsAuditExperienceOrbFixture.position(eye, new Vec3(0,0,2));
        assertEquals(new Vec3(9.4,20,33), p);
        assertEquals(p, GraphicsAuditExperienceOrbFixture.position(eye, new Vec3(0,0,1)));
        assertThrows(IllegalArgumentException.class,
            () -> GraphicsAuditExperienceOrbFixture.position(eye, Vec3.ZERO));
        assertThrows(IllegalArgumentException.class,
            () -> GraphicsAuditExperienceOrbFixture.position(eye, new Vec3(Double.NaN,0,0)));
    }

    @Test void unownedStateIsNotModified() {
        var state = new ExperienceOrbRenderState();
        state.ageInTicks = 17.25f;
        state.icon = 3;
        state.lightCoords = 123;
        GraphicsAuditExperienceOrbFixture.configureRenderState(null, state);
        assertEquals(17.25f, state.ageInTicks);
        assertEquals(3, state.icon);
        assertEquals(123, state.lightCoords);
    }
}
