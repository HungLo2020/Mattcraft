package net.sodium.client.render.chunk.compile.pipeline;

import net.sodium.client.render.chunk.terrain.material.DefaultMaterials;
import net.sodium.client.render.chunk.terrain.material.Material;
import net.sodium.client.render.chunk.terrain.material.parameters.AlphaCutoffParameter;
import net.sodium.client.render.chunk.terrain.material.parameters.MaterialParameters;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class NativeMaterialPassSnapshotTest {
    @Test void translucentToCutoutCarriesFrozenAlphaCutoffAndPreservesMipPolicy() {
        // Frozen BlockRenderer updates these bits alongside the chosen pass.
        int bits = NativeStaticBlockModelRegistry.materialBitsForPass(DefaultMaterials.TRANSLUCENT, 1);
        assertEquals(MaterialParameters.pack(AlphaCutoffParameter.ONE_TENTH, DefaultMaterials.TRANSLUCENT.mipped), bits);
        assertNotEquals(DefaultMaterials.TRANSLUCENT.bits(), bits);
    }

    @Test void otherMaterialAndPassCombinationsKeepTheirDeclaredBits() {
        for (Material material : new Material[] {DefaultMaterials.SOLID, DefaultMaterials.CUTOUT,
                DefaultMaterials.CUTOUT_MIPPED, DefaultMaterials.TRANSLUCENT, DefaultMaterials.TRIPWIRE}) {
            for (int pass = 0; pass <= 2; pass++) {
                if (material == DefaultMaterials.TRANSLUCENT && pass == 1) continue;
                assertEquals(material.bits(), NativeStaticBlockModelRegistry.materialBitsForPass(material, pass));
            }
        }
    }
}
