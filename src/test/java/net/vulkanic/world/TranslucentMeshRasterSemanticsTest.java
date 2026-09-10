package net.vulkanic.world;

import java.lang.reflect.Method;
import net.minecraft.client.renderer.RenderType;
import net.minecraft.resources.ResourceLocation;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class TranslucentMeshRasterSemanticsTest {
    @Test
    void solidModelMaterialsRemainOpaqueAndCutoutModelsRemainDistinct() throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        ResourceLocation texture = ResourceLocation.withDefaultNamespace("textures/entity/shield_base_nopattern.png");
        Method extract = RustGalWorldPrimitiveRenderer.class.getDeclaredMethod("modelMeshRenderSemantics", RenderType.class);
        extract.setAccessible(true);
        for (RenderType type : new RenderType[]{RenderType.entitySolid(texture),
                RenderType.entitySolidZOffsetForward(texture)}) {
            Object semantics = extract.invoke(null, type);
            assertEquals(RustGalWorldPrimitiveRenderer.MATERIAL_MODE_OPAQUE, field(semantics, "materialMode"));
            assertEquals(RustGalWorldPrimitiveRenderer.MATERIAL_ID_OPAQUE_TEXTURED, field(semantics, "materialId"));
            assertRasterMatchesPipeline(type);
        }
        for (RenderType type : new RenderType[]{RenderType.entityCutout(texture),
                RenderType.entityCutoutNoCull(texture), RenderType.entityCutoutNoCullZOffset(texture)}) {
            Object semantics = extract.invoke(null, type);
            assertEquals(RustGalWorldPrimitiveRenderer.MATERIAL_MODE_CUTOUT, field(semantics, "materialMode"));
            assertEquals(RustGalWorldPrimitiveRenderer.MATERIAL_ID_CUTOUT_TEXTURED, field(semantics, "materialId"));
            assertRasterMatchesPipeline(type);
        }
    }

    @Test
    void itemPipelineCarriesBlendPlusCutoutInsteadOfTerrainTranslucency() throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        Method extract = RustGalWorldPrimitiveRenderer.class.getDeclaredMethod("modelMeshRenderSemantics", RenderType.class);
        extract.setAccessible(true);
        Object semantics = extract.invoke(null, RenderType.itemEntityTranslucentCull(
            ResourceLocation.withDefaultNamespace("textures/block/stone.png")));
        assertEquals(RustGalWorldPrimitiveRenderer.MATERIAL_MODE_TRANSLUCENT_CUTOUT, field(semantics, "materialMode"));
        assertEquals(RustGalWorldPrimitiveRenderer.MATERIAL_ID_TRANSLUCENT_CUTOUT_TEXTURED, field(semantics, "materialId"));
        assertEquals(RustGalWorldPrimitiveRenderer.DEPTH_POLICY_TEST_WRITE, field(semantics, "depthPolicy"));
        assertEquals(RustGalWorldPrimitiveRenderer.CULL_BACK, field(semantics, "cullPolicy"));
    }

    @Test
    void itemCullingAndEntityDoubleSidednessRemainDistinct() throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        ResourceLocation texture = ResourceLocation.withDefaultNamespace("textures/block/glass.png");
        RenderType item = RenderType.itemEntityTranslucentCull(texture);
        RenderType entity = RenderType.entityTranslucent(texture);
        assertTrue(item.pipeline().isCull());
        assertTrue(item.pipeline().isWriteDepth());
        assertFalse(entity.pipeline().isCull());
        assertRasterMatchesPipeline(item);
        assertRasterMatchesPipeline(entity);
        assertRasterMatchesPipeline(RenderType.entityTranslucentEmissive(texture));
    }

    private static void assertRasterMatchesPipeline(RenderType type) throws Exception {
        Method extract = RustGalWorldPrimitiveRenderer.class.getDeclaredMethod("modelMeshRenderSemantics", RenderType.class);
        extract.setAccessible(true);
        Object record = extract.invoke(null, type);
        assertNotNull(record);
        assertEquals(type.pipeline().isCull() ? RustGalWorldPrimitiveRenderer.CULL_BACK : RustGalWorldPrimitiveRenderer.CULL_NONE,
            field(record, "cullPolicy"));
        assertEquals(type.pipeline().isWriteDepth() ? RustGalWorldPrimitiveRenderer.DEPTH_POLICY_TEST_WRITE
            : RustGalWorldPrimitiveRenderer.DEPTH_POLICY_TEST_NO_WRITE, field(record, "depthPolicy"));
    }

    private static int field(Object record, String name) throws Exception {
        Method accessor = record.getClass().getDeclaredMethod(name);
        accessor.setAccessible(true);
        return (int) accessor.invoke(record);
    }
}
