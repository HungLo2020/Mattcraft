package net.vulkanic.world;

import java.util.ArrayList;
import java.util.List;
import net.vulkanic.bridge.VulkanicGalBridge;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class ExperienceOrbExecutionReceiptTest {
    private static VulkanicGalBridge.WorldExperienceOrbInstanceRecord orb(long key, long generation, int entity) {
        return new VulkanicGalBridge.WorldExperienceOrbInstanceRecord(key, generation, 0,
            new org.joml.Matrix4f().get(new float[16]), new float[]{0,0,0,1}, entity);
    }

    @Test void copiesTypedResourceIdentitiesAndCapturedSubmission() {
        var source = new ArrayList<>(List.of(orb(123,7,42)));
        var receipt = RustGalWorldPrimitiveRenderer.ExperienceOrbExecutionDiagnostic.fromSubmittedOrbs(5,11,17,1,source);
        source.clear();
        assertEquals(5,receipt.deterministicFrameIndex());
        assertEquals(11,receipt.gameplayFrameId());
        assertEquals(17,receipt.submissionId());
        assertEquals(1,receipt.nativeOrbCount());
        assertTrue(receipt.nativeResourcesComplete());
        assertEquals(new RustGalWorldPrimitiveRenderer.ExperienceOrbResourceReceipt(123,7,42),
            receipt.nativeResources().getFirst());
        assertThrows(UnsupportedOperationException.class,()->receipt.nativeResources().clear());
    }

    @Test void aGenericQuadCannotClaimNativeOrbCoverage() {
        var receipt = RustGalWorldPrimitiveRenderer.ExperienceOrbExecutionDiagnostic.fromSubmittedOrbs(5,11,17,1,List.of());
        assertEquals(1,receipt.quads());
        assertEquals(0,receipt.nativeOrbCount());
        assertTrue(receipt.nativeResources().isEmpty());
    }

    @Test void diagnosticsAreBoundedAndTruncationIsExplicit() {
        var source = new ArrayList<VulkanicGalBridge.WorldExperienceOrbInstanceRecord>();
        for (int i=0;i<65;i++) source.add(orb(i+1,1,i));
        var receipt = RustGalWorldPrimitiveRenderer.ExperienceOrbExecutionDiagnostic.fromSubmittedOrbs(5,11,17,65,source);
        assertEquals(65,receipt.nativeOrbCount());
        assertEquals(64,receipt.nativeResources().size());
        assertFalse(receipt.nativeResourcesComplete());
        assertThrows(IllegalArgumentException.class,
            ()->RustGalWorldPrimitiveRenderer.ExperienceOrbExecutionDiagnostic.fromSubmittedOrbs(5,11,17,0,List.of(orb(1,1,1))));
    }
}
